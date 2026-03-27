#!/usr/bin/env python3
"""
Local eval: end-to-end QA evaluation using real remem data.

Same pipeline as LoCoMo eval but against your actual memories:
  1. Sample N memories from real DB
  2. LLM generates a question + gold answer per memory
  3. Search remem API for each question
  4. LLM generates answer from retrieved context
  5. LLM judges correctness
  6. Report accuracy (overall + by memory_type)

Usage:
  # Requires remem API running: remem api --port 5567
  python eval/local/run_local_eval.py
  python eval/local/run_local_eval.py --n 50 --model gpt-5.4
"""

import argparse
import json
import os
import sqlite3
import time
from collections import defaultdict
from pathlib import Path

import httpx

RESULTS_DIR = Path(__file__).parent / "results"

# ---------------------------------------------------------------------------
# LLM helpers (same as LoCoMo eval)
# ---------------------------------------------------------------------------

def _load_env_file(path):
    """Load key=value pairs from a .env file into os.environ (no override)."""
    if not os.path.isfile(path):
        return
    with open(path) as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#") or "=" not in line:
                continue
            k, v = line.split("=", 1)
            os.environ.setdefault(k.strip(), v.strip())


# Project root .env (two levels up from eval/local/)
_PROJECT_ENV = os.path.join(os.path.dirname(__file__), "..", "..", ".env")


def create_openai_client():
    from openai import OpenAI

    _load_env_file(_PROJECT_ENV)

    api_key = os.environ.get("OPENAI_API_KEY", "")
    base_url = os.environ.get("OPENAI_BASE_URL", None)
    if not api_key:
        raise ValueError(
            "OPENAI_API_KEY not set. Create .env in project root with:\n"
            "  OPENAI_API_KEY=your-key\n"
            "  OPENAI_BASE_URL=your-base-url  # optional\n"
            "  OPENAI_MODEL=gpt-4o            # optional"
        )
    kwargs = {"api_key": api_key}
    if base_url:
        kwargs["base_url"] = base_url
    return OpenAI(**kwargs)


def llm_generate(client, prompt, model="gpt-5.4", max_tokens=128):
    for attempt in range(3):
        try:
            resp = client.chat.completions.create(
                model=model,
                messages=[{"role": "user", "content": prompt}],
                temperature=0.0,
                max_tokens=max_tokens,
                timeout=60,
            )
            return resp.choices[0].message.content.strip()
        except Exception as e:
            print(f"    LLM error (attempt {attempt+1}): {e}")
            if attempt < 2:
                time.sleep(3)
    return "ERROR: LLM call failed"


# ---------------------------------------------------------------------------
# Step 1: Sample memories from real DB
# ---------------------------------------------------------------------------

def sample_memories(db_path, n, project_filter=None):
    """Sample N diverse memories from the real database."""
    conn = sqlite3.connect(db_path)
    conn.row_factory = sqlite3.Row

    where = "WHERE status = 'active' AND LENGTH(content) > 50"
    params = []
    if project_filter:
        where += " AND project = ?"
        params.append(project_filter)

    # Sample across types for diversity
    rows = conn.execute(f"""
        SELECT id, project, title, content, memory_type, created_at_epoch
        FROM memories {where}
        ORDER BY RANDOM()
        LIMIT ?
    """, params + [n * 2]).fetchall()  # over-fetch, then diversify
    conn.close()

    # Diversify: balance across memory_types
    by_type = defaultdict(list)
    for r in rows:
        by_type[r["memory_type"]].append(dict(r))

    samples = []
    per_type = max(n // max(len(by_type), 1), 2)
    for mtype, mems in by_type.items():
        samples.extend(mems[:per_type])
    # Fill remaining slots
    remaining = [m for m in rows if dict(m) not in samples]
    for m in remaining:
        if len(samples) >= n:
            break
        d = dict(m)
        if d not in samples:
            samples.append(d)

    return samples[:n]


# ---------------------------------------------------------------------------
# Step 2: Generate questions from memories
# ---------------------------------------------------------------------------

QUESTION_GEN_PROMPT = """\
Given this memory from a developer's work log, generate ONE question that \
someone would naturally ask to retrieve this information later. Also provide \
the expected answer (extracted directly from the memory content).

Memory type: {memory_type}
Memory content:
{content}

Requirements:
- Question should be natural (how a developer would actually ask)
- Answer should be a short phrase (3-15 words) directly from the content
- For decisions: ask "why did we choose X" or "what was decided about X"
- For discoveries: ask "what did we learn about X"
- For bugfixes: ask "what caused the X bug" or "how did we fix X"
- For preferences: ask "what does the user prefer about X"

Return ONLY JSON: {{"question": "...", "answer": "...", "category": "single-hop"}}
Category must be one of: single-hop, multi-hop, temporal, open-domain"""


def generate_qa_pairs(openai_client, memories, model):
    """Generate question-answer pairs from sampled memories."""
    qa_pairs = []
    for i, mem in enumerate(memories):
        content = mem["content"][:800]
        prompt = QUESTION_GEN_PROMPT.format(
            memory_type=mem["memory_type"],
            content=content,
        )
        text = llm_generate(openai_client, prompt, model=model, max_tokens=200)
        try:
            qa = json.loads(text)
            if "question" in qa and "answer" in qa:
                qa["memory_id"] = mem["id"]
                qa["memory_type"] = mem["memory_type"]
                qa["project"] = mem["project"]
                qa["memory_content"] = content
                qa_pairs.append(qa)
                print(f"  [{i+1}/{len(memories)}] {mem['memory_type']}: {qa['question'][:60]}...")
        except (json.JSONDecodeError, TypeError):
            print(f"  [{i+1}/{len(memories)}] SKIP (parse error)")
    return qa_pairs


# ---------------------------------------------------------------------------
# Step 3: Search remem
# ---------------------------------------------------------------------------

def search_remem(http, base_url, project, question, top_k=20):
    """Search remem API."""
    try:
        resp = http.get(
            f"{base_url}/api/v1/search",
            params={"query": question, "project": project, "limit": top_k},
            timeout=30,
        )
        if resp.status_code == 200:
            data = resp.json()
            return data.get("data", []) if isinstance(data, dict) else data
    except Exception as e:
        print(f"    Search error: {e}")
    return []


# ---------------------------------------------------------------------------
# Step 4: Answer generation (same as LoCoMo)
# ---------------------------------------------------------------------------

ANSWER_PROMPT = """\
You are answering questions about a developer's past work and decisions.
Use the retrieved memory excerpts below as your primary source.

Retrieved memories:
{context}

Question: {question}

Answer in a short phrase (a few words). Be concise and precise."""


def generate_answer(openai_client, question, memories, model):
    if not memories:
        return "No information available"
    context_str = "\n".join(
        f"- {m.get('content', m.get('text', ''))[:300]}" for m in memories
    )
    prompt = ANSWER_PROMPT.format(context=context_str, question=question)
    return llm_generate(openai_client, prompt, model=model, max_tokens=64)


# ---------------------------------------------------------------------------
# Step 5: Judge (same as LoCoMo)
# ---------------------------------------------------------------------------

JUDGE_PROMPT = """\
Label the generated answer as 'CORRECT' or 'WRONG'.

Question: {question}
Gold answer: {gold_answer}
Generated answer: {generated_answer}

Be generous: if the generated answer touches the same topic and key facts \
as the gold answer, count it as CORRECT.

Return JSON: {{"label": "CORRECT"}} or {{"label": "WRONG"}}"""


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

CATEGORY_NAMES = {
    "single-hop": "Single-hop",
    "multi-hop": "Multi-hop",
    "temporal": "Temporal",
    "open-domain": "Open-domain",
}


def run_pipeline(http, base_url, qa_pairs, openai_client, model, top_k):
    print(f"\n=== QA Pipeline ({len(qa_pairs)} questions, top_k={top_k}) ===")

    results = []
    type_scores = defaultdict(list)
    cat_scores = defaultdict(list)
    total = len(qa_pairs)

    for i, qa in enumerate(qa_pairs):
        question = qa["question"]
        gold = qa["answer"]
        project = qa["project"]
        mtype = qa["memory_type"]
        category = qa.get("category", "single-hop")

        memories = search_remem(http, base_url, project, question, top_k)
        generated = generate_answer(openai_client, question, memories, model)
        score = judge_answer(openai_client, question, gold, generated, model)

        # Check if the source memory was retrieved
        source_found = any(
            m.get("id") == qa["memory_id"] for m in memories
        )

        type_scores[mtype].append(score)
        cat_scores[category].append(score)

        results.append({
            "question": question,
            "gold_answer": gold,
            "generated_answer": generated,
            "score": score,
            "source_retrieved": source_found,
            "memory_type": mtype,
            "category": category,
            "project": project,
            "retrieved_count": len(memories),
        })

        status = "OK" if score else "MISS"
        src = "SRC" if source_found else "---"
        running = sum(s for scores in cat_scores.values() for s in scores)
        total_scored = sum(len(scores) for scores in cat_scores.values())
        acc = running / total_scored if total_scored else 0
        print(f"  [{i+1}/{total}] {status} {src} acc={acc:.1%} type={mtype} q={question[:50]}...")

    return results, dict(type_scores), dict(cat_scores)


def print_report(type_scores, cat_scores):
    print("\n" + "=" * 60)
    print("  Local Eval Results — remem (real data)")
    print("=" * 60)

    all_scores = []

    if cat_scores:
        print("\n  By category:")
        for cat in sorted(cat_scores):
            scores = cat_scores[cat]
            if not scores:
                continue
            acc = sum(scores) / len(scores)
            name = CATEGORY_NAMES.get(cat, cat)
            print(f"    {name:>12}: {acc:.1%}  ({sum(scores)}/{len(scores)})")
            all_scores.extend(scores)

    if type_scores:
        print("\n  By memory type:")
        for mtype in sorted(type_scores):
            scores = type_scores[mtype]
            if not scores:
                continue
            acc = sum(scores) / len(scores)
            print(f"    {mtype:>12}: {acc:.1%}  ({sum(scores)}/{len(scores)})")
            if not all_scores:
                all_scores.extend(scores)

    if all_scores:
        overall = sum(all_scores) / len(all_scores)
        print(f"\n  {'Overall':>14}: {overall:.1%}  ({sum(all_scores)}/{len(all_scores)})")
    print("=" * 60)


def main():
    parser = argparse.ArgumentParser(description="Local eval: real-data QA benchmark for remem")
    parser.add_argument("--remem-url", default="http://127.0.0.1:5567")
    parser.add_argument("--db", default=str(Path.home() / ".remem" / "remem.db"))
    _load_env_file(_PROJECT_ENV)
    parser.add_argument(
        "--model",
        default=os.environ.get("OPENAI_MODEL", "gpt-5.4"),
    )
    parser.add_argument("--n", type=int, default=30, help="Number of memories to sample")
    parser.add_argument("--top-k", type=int, default=20)
    parser.add_argument("--project", default=None, help="Filter by project")
    parser.add_argument("--skip-gen", action="store_true", help="Reuse cached QA pairs")
    args = parser.parse_args()

    RESULTS_DIR.mkdir(parents=True, exist_ok=True)
    qa_cache = RESULTS_DIR / "qa_pairs.json"

    openai_client = create_openai_client()
    http = httpx.Client()

    if args.skip_gen and qa_cache.exists():
        print(f"Loading cached QA pairs from {qa_cache}")
        with open(qa_cache) as f:
            qa_pairs = json.load(f)
        print(f"  {len(qa_pairs)} QA pairs\n")
    else:
        print(f"=== Step 1: Sample {args.n} memories from {args.db} ===")
        memories = sample_memories(args.db, args.n, args.project)
        print(f"  Sampled {len(memories)} memories across {len(set(m['memory_type'] for m in memories))} types\n")

        print(f"=== Step 2: Generate QA pairs (model={args.model}) ===")
        qa_pairs = generate_qa_pairs(openai_client, memories, args.model)
        print(f"  Generated {len(qa_pairs)} QA pairs\n")

        with open(qa_cache, "w") as f:
            json.dump(qa_pairs, f, indent=2, ensure_ascii=False)

    t0 = time.time()
    results, type_scores, cat_scores = run_pipeline(
        http, args.remem_url, qa_pairs, openai_client, args.model, args.top_k
    )
    elapsed = time.time() - t0

    print_report(type_scores, cat_scores)
    print(f"\n  Time: {elapsed:.1f}s")

    # Source retrieval rate
    src_found = sum(1 for r in results if r["source_retrieved"])
    print(f"  Source memory in top-{args.top_k}: {src_found}/{len(results)} ({src_found/max(len(results),1):.1%})")

    with open(RESULTS_DIR / "local_eval_results.json", "w") as f:
        json.dump(results, f, indent=2, ensure_ascii=False)
    print(f"\n  Results saved to {RESULTS_DIR / 'local_eval_results.json'}")


if __name__ == "__main__":
    main()
