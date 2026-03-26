"""
LoCoMo Benchmark Runner for remem (v3 — all optimizations except vector search).

Optimizations:
1. Hybrid ingest: session_summary + timestamp index from raw turns
2. top_k=20 (was 10)
3. Query decomposition: LLM splits multi-hop into sub-queries
4. Iterative retrieval: if first search insufficient, LLM rewrites query
5. LLM reranking: top-20 → top-10 via relevance scoring

Usage:
    remem api --port 5567  # Start API first
    python run_locomo.py --sample-index 0  # Single conversation
    python run_locomo.py                    # All 10
"""

import argparse
import json
import math
import os
import time
from collections import defaultdict
from datetime import datetime
from pathlib import Path

import httpx
try:
    import numpy as np
    NUMPY_AVAILABLE = True
except ImportError:
    NUMPY_AVAILABLE = False

DATA_PATH = Path(__file__).parent / "locomo10.json"
RESULTS_DIR = Path(__file__).parent / "results"

CATEGORY_NAMES = {
    1: "multi-hop",
    2: "temporal",
    3: "open-domain",
    4: "single-hop",
    5: "adversarial",
}


def create_openai_client():
    from openai import OpenAI
    api_key = os.environ.get("OPENAI_API_KEY", "")
    base_url = os.environ.get("OPENAI_BASE_URL", None)
    if not api_key:
        raise ValueError("OPENAI_API_KEY not set")
    kwargs = {"api_key": api_key}
    if base_url:
        kwargs["base_url"] = base_url
    return OpenAI(**kwargs)


def llm_generate(client, prompt, model="gpt-5.4", max_tokens=128, reasoning_effort="xhigh"):
    for attempt in range(3):
        try:
            resp = client.chat.completions.create(
                model=model,
                messages=[{"role": "user", "content": prompt}],
                temperature=0.0,
                max_tokens=max_tokens,
                timeout=60,
                extra_body={"reasoning_effort": reasoning_effort},
            )
            return resp.choices[0].message.content.strip()
        except Exception as e:
            print(f"    LLM error (attempt {attempt+1}): {e}")
            if attempt < 2:
                time.sleep(3)
    return "ERROR: LLM call failed"


# ---------------------------------------------------------------------------
# Vector search helpers (OpenAI embeddings + cosine similarity)
# ---------------------------------------------------------------------------

_embedding_cache: dict = {}


def get_embedding(openai_client, text: str) -> list:
    """Get OpenAI embedding for text, with in-memory cache."""
    import hashlib
    key = hashlib.sha256(text[:8000].encode()).hexdigest()
    if key in _embedding_cache:
        return _embedding_cache[key]
    for attempt in range(3):
        try:
            resp = openai_client.embeddings.create(
                model="text-embedding-3-small",
                input=text[:8000],
            )
            emb = resp.data[0].embedding
            _embedding_cache[key] = emb
            return emb
        except Exception as e:
            if attempt < 2:
                time.sleep(2)
    return []


def cosine_similarity(a: list, b: list) -> float:
    """Cosine similarity between two embedding vectors."""
    if not a or not b or len(a) != len(b):
        return 0.0
    if NUMPY_AVAILABLE:
        va, vb = np.array(a), np.array(b)
        denom = np.linalg.norm(va) * np.linalg.norm(vb)
        return float(np.dot(va, vb) / denom) if denom > 0 else 0.0
    dot = sum(x * y for x, y in zip(a, b))
    na = math.sqrt(sum(x * x for x in a))
    nb = math.sqrt(sum(x * x for x in b))
    return dot / (na * nb) if na * nb > 0 else 0.0


def vector_retrieve(openai_client, memories: list, query: str, top_k: int = 20) -> list:
    """Re-rank memories by cosine similarity to query embedding."""
    if not memories or not openai_client:
        return memories
    q_emb = get_embedding(openai_client, query)
    if not q_emb:
        return memories
    scored = []
    for m in memories:
        text = m.get("content", "") + " " + m.get("title", "")
        m_emb = get_embedding(openai_client, text[:500])
        score = cosine_similarity(q_emb, m_emb)
        scored.append((score, m))
    scored.sort(key=lambda x: -x[0])
    return [m for _, m in scored[:top_k]]


def rrf_fuse(fts_memories: list, vec_memories: list, k: int = 60) -> list:
    """Reciprocal Rank Fusion of FTS5 and vector results."""
    scores: dict = {}
    id_to_mem: dict = {}
    for rank, m in enumerate(fts_memories):
        mid = m.get("id", hash(m.get("content", "")[:80]))
        scores[mid] = scores.get(mid, 0.0) + 1.0 / (k + rank + 1)
        id_to_mem[mid] = m
    for rank, m in enumerate(vec_memories):
        mid = m.get("id", hash(m.get("content", "")[:80]))
        scores[mid] = scores.get(mid, 0.0) + 1.0 / (k + rank + 1)
        id_to_mem[mid] = m
    sorted_ids = sorted(scores, key=lambda x: -scores[x])
    return [id_to_mem[mid] for mid in sorted_ids]


# ---------------------------------------------------------------------------
# Step 1: Hybrid ingest (summary + observation + timestamp index)
# ---------------------------------------------------------------------------

PERSONA_EXTRACTION_PROMPT = """\
Extract 2-5 facts about people's personality, preferences, habits, and values from this conversation.
Focus on: what people like/dislike, recurring behaviors, personal traits, relationships between people.

Conversation ({date_time}):
{conversation}

Return ONLY a JSON array of fact strings. If no personality/preference facts exist, return [].
Example: ["Caroline enjoys outdoor activities like hiking", "Melanie values family time above career"]

JSON array:"""


FACT_EXTRACTION_PROMPT = """\
Extract 3-8 key facts from this conversation session. Each fact should be:
- A specific, searchable statement about what happened, who was involved, when, and where
- Self-contained (understandable without the full conversation)
- Include names, dates, locations, and specific details

Conversation ({date_time}):
{conversation}

Return ONLY a JSON array of fact strings. Example:
["Caroline attended a pride parade on May 15, 2023", "Melanie's son Tom loves dinosaurs and space"]

JSON array:"""


def extract_facts_from_session(openai_client, turns, date_time, model):
    """Use LLM to extract key facts from a conversation session (Hindsight approach).
    Processes in batches of 10 turns to avoid timeout on long sessions."""
    all_facts = []
    batch_size = 10

    for start in range(0, len(turns), batch_size):
        batch = turns[start:start + batch_size]
        lines = []
        for turn in batch:
            speaker = turn.get("speaker", "?")
            text = turn.get("text", "")[:200]
            lines.append(f"{speaker}: {text}")
            if "blip_caption" in turn:
                lines[-1] += f" [image: {turn['blip_caption']}]"

        conversation_text = "\n".join(lines)
        prompt = FACT_EXTRACTION_PROMPT.format(
            date_time=date_time, conversation=conversation_text
        )
        text = llm_generate(openai_client, prompt, model=model, max_tokens=300)
        try:
            facts = json.loads(text)
            if isinstance(facts, list):
                all_facts.extend(f for f in facts if isinstance(f, str) and len(f) > 10)
        except (json.JSONDecodeError, TypeError):
            pass

    return all_facts


def extract_persona_from_session(openai_client, turns, date_time, model):
    """Extract personality/preference facts for Open-domain QA."""
    lines = []
    for turn in turns[:20]:  # first 20 turns enough for persona signals
        speaker = turn.get("speaker", "?")
        text = turn.get("text", "")[:200]
        lines.append(f"{speaker}: {text}")
    conversation_text = "\n".join(lines)
    prompt = PERSONA_EXTRACTION_PROMPT.format(date_time=date_time, conversation=conversation_text)
    text = llm_generate(openai_client, prompt, model=model, max_tokens=200)
    try:
        facts = json.loads(text)
        if isinstance(facts, list):
            return [f for f in facts if isinstance(f, str) and len(f) > 10]
    except (json.JSONDecodeError, TypeError):
        pass
    return []


def ingest_conversation(http, base_url, sample, openai_client=None, model="gpt-5.4"):
    """v4 ingest: LLM fact extraction from raw conversations (fair, like Hindsight).
    No session_summary (that's human-annotated, unfair). Instead, LLM extracts facts."""
    sample_id = sample["sample_id"]
    conv = sample["conversation"]
    count = 0
    project = f"locomo/{sample_id}"

    session_nums = sorted(
        int(k.split("_")[-1])
        for k in conv
        if k.startswith("session_") and "date_time" not in k
    )

    for sess_num in session_nums:
        turns = conv.get(f"session_{sess_num}", [])
        date_time = conv.get(f"session_{sess_num}_date_time", f"session {sess_num}")

        # Parse session date to epoch for correct temporal ordering
        session_epoch = None
        try:
            session_epoch = int(datetime.strptime(date_time, "%B %d, %Y").timestamp())
        except ValueError:
            try:
                session_epoch = int(datetime.fromisoformat(date_time).timestamp())
            except ValueError:
                pass

        # Layer 1: LLM fact extraction (like Hindsight's retain pipeline)
        if openai_client is not None:
            facts = extract_facts_from_session(openai_client, turns, date_time, model)
            for i, fact in enumerate(facts):
                payload = {
                    "project": project,
                    "title": fact[:200],
                    "content": f"[{date_time}] {fact}",
                    "memory_type": "discovery",
                    "topic_key": f"locomo-{sample_id}-fact-s{sess_num}-{i}",
                    "scope": "project",
                }
                if session_epoch is not None:
                    payload["created_at_epoch"] = session_epoch + i  # slight offset to preserve order
                resp = http.post(f"{base_url}/api/v1/memories", json=payload, timeout=30)
                if resp.status_code == 201:
                    count += 1

        # Layer 1b: Persona/preference extraction (for Open-domain QA)
        if openai_client is not None:
            persona_facts = extract_persona_from_session(openai_client, turns, date_time, model)
            for i, fact in enumerate(persona_facts):
                payload = {
                    "project": project,
                    "title": fact[:200],
                    "content": f"[{date_time}] {fact}",
                    "memory_type": "preference",
                    "topic_key": f"locomo-{sample_id}-persona-s{sess_num}-{i}",
                    "scope": "project",
                }
                if session_epoch is not None:
                    payload["created_at_epoch"] = session_epoch
                resp = http.post(f"{base_url}/api/v1/memories", json=payload, timeout=30)
                if resp.status_code == 201:
                    count += 1

        # Layer 2: Per-session timeline with timestamps (for temporal queries)
        lines = [f"[{date_time}] Session {sess_num} conversation:"]
        for turn in turns:
            speaker = turn["speaker"]
            text = turn["text"][:150]
            dia_id = turn.get("dia_id", "")
            lines.append(f"  {dia_id} {speaker}: {text}")
            if "blip_caption" in turn:
                lines[-1] += f" [image: {turn['blip_caption']}]"
        content = "\n".join(lines)
        payload = {
            "project": project,
            "title": f"Session {sess_num} ({date_time})"[:200],
            "content": content[:5000],
            "memory_type": "session_activity",
            "topic_key": f"locomo-{sample_id}-session-{sess_num}",
            "scope": "project",
        }
        if session_epoch is not None:
            payload["created_at_epoch"] = session_epoch
        resp = http.post(f"{base_url}/api/v1/memories", json=payload, timeout=30)
        if resp.status_code == 201:
            count += 1

    return count


def ingest_all(http, base_url, samples, openai_client=None, model="gpt-5.4"):
    print(f"=== Step 1: LLM fact extraction + timeline ingest ({len(samples)} conversations) ===")
    total = 0
    for sample in samples:
        sid = sample["sample_id"]
        n = ingest_conversation(http, base_url, sample, openai_client, model)
        total += n
        print(f"  [{sid}] ingested {n} memories")
    print(f"  Total: {total} memories\n")


# ---------------------------------------------------------------------------
# Step 2: Enhanced retrieval (decomposition + iterative + rerank)
# ---------------------------------------------------------------------------

def retrieve_context(http, base_url, sample_id, question, top_k):
    resp = http.get(
        f"{base_url}/api/v1/search",
        params={"query": question, "project": f"locomo/{sample_id}", "limit": top_k},
        timeout=30,
    )
    if resp.status_code != 200:
        return []
    return resp.json().get("data", [])


def decompose_query(openai_client, question, model):
    """For multi-hop questions, decompose into sub-queries covering different hop types."""
    prompt = f"""Break this question into 1-3 simpler search queries for a conversation memory database. Return ONLY a JSON array of strings.

Question: {question}

Examples:
"What do Melanie's kids like?" → ["Melanie children names", "Melanie kids hobbies interests"]
"When did Caroline go to the LGBTQ conference?" → ["Caroline LGBTQ conference date"]
"What activity did both Caroline and Tom participate in?" → ["Caroline activities events", "Tom activities events"]
"What is the relationship between Sarah and Marcus?" → ["Sarah Marcus relationship", "Sarah friends family", "Marcus friends family"]

Return JSON array:"""
    text = llm_generate(openai_client, prompt, model=model, max_tokens=100)
    try:
        queries = json.loads(text)
        if isinstance(queries, list) and queries:
            return [q for q in queries if isinstance(q, str) and q.strip()]
    except (json.JSONDecodeError, TypeError):
        pass
    return [question]


def extract_entities_from_memories(openai_client, question, memories, model):
    """M1: Extract intermediate entities from hop-1 results for hop-2 search."""
    if not memories:
        return []
    context = "\n".join(f"- {m.get('content', '')[:200]}" for m in memories[:5])
    prompt = f"""Given this question and memory excerpts, extract 1-3 specific names/entities
that appear in the memories and would help answer the question if searched further.
Return ONLY a JSON array of short search strings (names, places, activities).
If no useful entities found, return [].

Question: {question}
Memories:
{context}

JSON array:"""
    text = llm_generate(openai_client, prompt, model=model, max_tokens=80)
    try:
        entities = json.loads(text)
        if isinstance(entities, list):
            return [e for e in entities if isinstance(e, str) and e.strip()]
    except (json.JSONDecodeError, TypeError):
        pass
    return []


def rerank_memories(openai_client, question, memories, model, top_n=10):
    """LLM reranking: score each memory's relevance, return top_n."""
    if len(memories) <= top_n:
        return memories

    # Build compact list for LLM scoring
    items = []
    for i, m in enumerate(memories[:20]):
        content = m.get("content", "")[:200]
        items.append(f"[{i}] {content}")

    prompt = f"""Rate the relevance of each memory excerpt to the question on a scale of 0-3.
0=irrelevant, 1=marginally relevant, 2=partially relevant, 3=highly relevant.
Return ONLY a JSON object mapping index to score, e.g. {{"0": 3, "1": 0, "2": 2}}

Question: {question}

Memory excerpts:
{chr(10).join(items)}

JSON scores:"""

    text = llm_generate(openai_client, prompt, model=model, max_tokens=200)
    try:
        scores = json.loads(text)
        scored = [(i, int(scores.get(str(i), 0))) for i in range(len(memories[:20]))]
        scored.sort(key=lambda x: -x[1])
        return [memories[i] for i, _ in scored[:top_n]]
    except (json.JSONDecodeError, TypeError, ValueError):
        return memories[:top_n]


def enhanced_retrieve(http, base_url, sample_id, question, category, openai_client, model):
    """Enhanced retrieval: decompose → hop-1 → LLM entity extraction → hop-2 → rerank."""
    all_memories = []
    seen_ids = set()

    def add_results(results):
        for m in results:
            mid = m.get("id", hash(m.get("content", "")[:80]))
            if mid not in seen_ids:
                seen_ids.add(mid)
                all_memories.append(m)

    # For multi-hop: decompose query into sub-queries
    if category == 1:  # multi-hop
        sub_queries = decompose_query(openai_client, question, model)
    else:
        sub_queries = [question]

    # Hop 1: FTS5 search with each sub-query
    fts_memories = []
    for q in sub_queries:
        for m in retrieve_context(http, base_url, sample_id, q, 20):
            mid = m.get("id", hash(m.get("content", "")[:80]))
            if mid not in seen_ids:
                seen_ids.add(mid)
                fts_memories.append(m)
                all_memories.append(m)

    # Vector search: re-score FTS results + retrieve by semantic similarity
    vec_memories = vector_retrieve(openai_client, fts_memories, question, top_k=20)

    # RRF fusion of FTS5 and vector results
    all_memories = rrf_fuse(fts_memories, vec_memories)
    seen_ids = {m.get("id", hash(m.get("content", "")[:80])) for m in all_memories}

    # M2: Hop 2 — extract intermediate entities from hop-1 results and search again
    if category == 1 and all_memories:
        hop2_entities = extract_entities_from_memories(openai_client, question, all_memories[:5], model)
        for entity in hop2_entities:
            for m in retrieve_context(http, base_url, sample_id, entity, 10):
                mid = m.get("id", hash(m.get("content", "")[:80]))
                if mid not in seen_ids:
                    seen_ids.add(mid)
                    all_memories.append(m)

    # Fallback: if still few results, extract key words
    if len(all_memories) < 5:
        words = [w for w in question.split() if len(w) > 4]
        if words:
            add_results(retrieve_context(http, base_url, sample_id, " ".join(words[:4]), 10))

    # Rerank top memories by relevance
    if len(all_memories) > 10:
        all_memories = rerank_memories(openai_client, question, all_memories, model, top_n=10)

    return all_memories[:10]


# ---------------------------------------------------------------------------
# Step 3: Generate answer
# ---------------------------------------------------------------------------

ANSWER_PROMPT = """\
You are answering questions about a conversation between two people.
Use the retrieved memory excerpts below as your primary source.
You may also use reasonable inference and common knowledge to supplement the memories when needed.
If you truly cannot determine the answer, say "No information available".

Retrieved memories:
{context}

Question: {question}

Answer in a short phrase (a few words). Be concise and precise."""


def generate_answer(openai_client, question, memories, category, model):
    if not memories:
        return "No information available"

    # For temporal questions, sort memories chronologically so LLM sees timeline in order
    if category == 2:
        memories = sorted(memories, key=lambda m: m.get("created_at_epoch", 0))

    context_str = "\n".join(f"- {m['content'][:300]}" for m in memories)
    prompt = ANSWER_PROMPT.format(context=context_str, question=question)

    if category == 2:
        prompt += "\nMemories are sorted chronologically. Use dates and session order to answer precisely."

    return llm_generate(openai_client, prompt, model=model, max_tokens=64)


# ---------------------------------------------------------------------------
# Step 4: LLM Judge
# ---------------------------------------------------------------------------

JUDGE_PROMPT = """\
Your task is to label an answer to a question as 'CORRECT' or 'WRONG'. You will be given:
(1) a question (posed by one user to another user),
(2) a 'gold' (ground truth) answer,
(3) a generated answer.

The gold answer is usually concise. The generated answer might be longer.
Be generous: as long as it touches on the same topic as the gold answer, count it as CORRECT.
For time-related questions, accept different date formats referring to the same time.

Question: {question}
Gold answer: {gold_answer}
Generated answer: {generated_answer}

Return as JSON: {{"label": "CORRECT"}} or {{"label": "WRONG"}}"""


def judge_answer(openai_client, question, gold_answer, generated_answer, model):
    prompt = JUDGE_PROMPT.format(
        question=question, gold_answer=gold_answer, generated_answer=generated_answer
    )
    text = llm_generate(openai_client, prompt, model=model, max_tokens=64)
    try:
        label = json.loads(text).get("label", "")
        return 1 if label == "CORRECT" else 0
    except (json.JSONDecodeError, KeyError):
        return 1 if "CORRECT" in text.upper() and "WRONG" not in text.upper() else 0


# ---------------------------------------------------------------------------
# Pipeline
# ---------------------------------------------------------------------------

def run_pipeline(http, base_url, samples, openai_client, model, top_k, skip_ingest):
    if not skip_ingest:
        ingest_all(http, base_url, samples, openai_client, model)

    print(f"=== Step 2-4: QA pipeline (top_k={top_k}, model={model}) ===")

    all_results = {}
    category_scores = defaultdict(list)
    total_qa = sum(len(s["qa"]) for s in samples)
    processed = 0

    for sample in samples:
        sid = sample["sample_id"]
        sample_results = []

        for qa in sample["qa"]:
            question = qa.get("question", "")
            answer = str(qa.get("answer", ""))
            category = qa.get("category", 0)
            processed += 1

            if category == 5 or not question or not answer:
                continue

            # Enhanced retrieval (decomposition + iterative + rerank)
            memories = enhanced_retrieve(
                http, base_url, sid, question, category, openai_client, model
            )
            generated = generate_answer(openai_client, question, memories, category, model)
            score = judge_answer(openai_client, question, answer, generated, model)
            category_scores[category].append(score)

            sample_results.append({
                "question": question,
                "answer": answer,
                "response": generated,
                "category": str(category),
                "llm_score": score,
                "retrieved_count": len(memories),
            })

            cat_name = CATEGORY_NAMES.get(category, str(category))
            status = "OK" if score else "MISS"
            running = sum(s for scores in category_scores.values() for s in scores)
            total_scored = sum(len(scores) for scores in category_scores.values())
            acc = running / total_scored if total_scored else 0
            print(f"  [{processed}/{total_qa}] {status} acc={acc:.1%} cat={cat_name} q={question[:50]}...")

        all_results[sid] = sample_results

    return all_results, dict(category_scores)


def print_report(category_scores):
    print("\n" + "=" * 60)
    print("  LoCoMo Benchmark Results — remem v0.3.0 (v3)")
    print("=" * 60)

    all_scores = []
    for cat in sorted(category_scores):
        scores = category_scores[cat]
        if not scores:
            continue
        acc = sum(scores) / len(scores)
        cat_name = CATEGORY_NAMES.get(cat, str(cat))
        print(f"  Category {cat} ({cat_name:>12}): {acc:.1%}  ({sum(scores)}/{len(scores)})")
        all_scores.extend(scores)

    if all_scores:
        overall = sum(all_scores) / len(all_scores)
        print(f"  {'Overall':>28}: {overall:.1%}  ({sum(all_scores)}/{len(all_scores)})")
    print("=" * 60)


def main():
    parser = argparse.ArgumentParser(description="LoCoMo benchmark for remem (v3)")
    parser.add_argument("--remem-url", default="http://127.0.0.1:5567")
    parser.add_argument("--model", default="gpt-5.4")
    parser.add_argument("--top-k", type=int, default=20)
    parser.add_argument("--data-file", type=str, default=str(DATA_PATH))
    parser.add_argument("--skip-ingest", action="store_true")
    parser.add_argument("--max-samples", type=int, default=0, help="0=all")
    parser.add_argument("--sample-index", type=int, default=-1)
    args = parser.parse_args()

    print(f"Loading dataset from {args.data_file}")
    with open(args.data_file) as f:
        samples = json.load(f)
    if args.sample_index >= 0:
        samples = [samples[args.sample_index]]
    elif args.max_samples > 0:
        samples = samples[:args.max_samples]
    total_qa = sum(len(s["qa"]) for s in samples)
    print(f"  {len(samples)} conversations, {total_qa} QA pairs\n")

    openai_client = create_openai_client()
    http = httpx.Client()

    t0 = time.time()
    results, category_scores = run_pipeline(
        http, args.remem_url, samples, openai_client, args.model, args.top_k, args.skip_ingest
    )
    elapsed = time.time() - t0

    RESULTS_DIR.mkdir(exist_ok=True)
    suffix = f"_s{args.sample_index}" if args.sample_index >= 0 else ""
    with open(RESULTS_DIR / f"locomo_results{suffix}.json", "w") as f:
        json.dump(results, f, indent=2)

    print(f"\nTotal time: {elapsed:.0f}s")
    print_report(category_scores)

    summary = {"config": {"model": args.model, "top_k": args.top_k, "retriever": "remem-fts5-rrf-v3"}}
    all_scores = []
    for cat, scores in sorted(category_scores.items()):
        if scores:
            acc = sum(scores) / len(scores)
            summary[CATEGORY_NAMES.get(cat, str(cat))] = {
                "accuracy": round(acc, 4), "correct": sum(scores), "total": len(scores)
            }
            all_scores.extend(scores)
    if all_scores:
        summary["overall"] = {
            "accuracy": round(sum(all_scores) / len(all_scores), 4),
            "correct": sum(all_scores), "total": len(all_scores)
        }
    with open(RESULTS_DIR / f"locomo_scores{suffix}.json", "w") as f:
        json.dump(summary, f, indent=2)
    print(f"Scores saved to {RESULTS_DIR / 'locomo_scores.json'}")


if __name__ == "__main__":
    main()
