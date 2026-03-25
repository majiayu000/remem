"""
LoCoMo Benchmark Runner for remem.

Pipeline: ingest conversations -> retrieve per QA -> generate answer -> LLM judge.
Uses OpenAI SDK for generation + judging (same as Mem0 evaluation).

Usage:
    # Start remem API first: remem api --port 5567
    python run_locomo.py                          # Full run (all 10 conversations)
    python run_locomo.py --max-samples 1          # Quick test (1 conversation)
    python run_locomo.py --skip-ingest            # Skip ingestion if already done
"""

import argparse
import json
import os
import time
from collections import defaultdict
from pathlib import Path

import httpx

DATA_PATH = Path(__file__).parent / "locomo10.json"
RESULTS_DIR = Path(__file__).parent / "results"

CATEGORY_NAMES = {
    1: "multi-hop",
    2: "temporal",
    3: "open-domain",
    4: "single-hop",
    5: "adversarial",
}


# ---------------------------------------------------------------------------
# OpenAI client (compatible with proxy endpoints)
# ---------------------------------------------------------------------------

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
                time.sleep(2)
    return "ERROR: LLM call failed"


# ---------------------------------------------------------------------------
# Step 1: Ingest conversations into remem
# ---------------------------------------------------------------------------

def ingest_conversation(http, base_url, sample):
    sample_id = sample["sample_id"]
    conv = sample["conversation"]
    count = 0

    session_nums = sorted(
        int(k.split("_")[-1])
        for k in conv
        if k.startswith("session_") and "date_time" not in k
    )

    for sess_num in session_nums:
        session_key = f"session_{sess_num}"
        date_key = f"session_{sess_num}_date_time"
        date_time = conv.get(date_key, f"session {sess_num}")
        turns = conv.get(session_key, [])

        for turn in turns:
            speaker = turn["speaker"]
            text = turn["text"]
            dia_id = turn.get("dia_id", "")

            content = f"[{date_time}] {speaker}: {text}"
            if "blip_caption" in turn:
                content += f" [shared image: {turn['blip_caption']}]"

            title = f"{speaker} - session {sess_num} ({dia_id})"

            resp = http.post(
                f"{base_url}/api/v1/memories",
                json={
                    "project": f"locomo/{sample_id}",
                    "title": title,
                    "content": content,
                    "memory_type": "session_activity",
                    "topic_key": f"locomo-{sample_id}-{dia_id}",
                    "scope": "project",
                },
                timeout=30,
            )
            if resp.status_code == 201:
                count += 1

    return count


def ingest_all(http, base_url, samples):
    print(f"=== Step 1: Ingesting {len(samples)} conversations ===")
    total = 0
    for sample in samples:
        sid = sample["sample_id"]
        n = ingest_conversation(http, base_url, sample)
        total += n
        print(f"  [{sid}] ingested {n} turns")
    print(f"  Total: {total} memories\n")


# ---------------------------------------------------------------------------
# Step 2: Retrieve relevant memories
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


# ---------------------------------------------------------------------------
# Step 3: Generate answer
# ---------------------------------------------------------------------------

ANSWER_PROMPT = """\
You are answering questions about a conversation between two people.
Use ONLY the retrieved memory excerpts below to answer. If the information is not available, say "No information available".

Retrieved memories:
{context}

Question: {question}

Answer in a short phrase (a few words). Be concise and precise."""


def generate_answer(openai_client, question, memories, category, model):
    if not memories:
        return "No information available"

    context_str = "\n".join(f"- {m['content']}" for m in memories)
    prompt = ANSWER_PROMPT.format(context=context_str, question=question)

    if category == 2:
        prompt += "\nUse dates from the memories to answer."

    return llm_generate(openai_client, prompt, model=model, max_tokens=64)


# ---------------------------------------------------------------------------
# Step 4: LLM Judge (same prompt as Mem0 evaluation)
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
        ingest_all(http, base_url, samples)

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

            # Skip category 5 (adversarial) — same as Mem0
            if category == 5 or not question or not answer:
                continue

            memories = retrieve_context(http, base_url, sid, question, top_k)
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
    print("  LoCoMo Benchmark Results — remem v0.3.0")
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
    parser = argparse.ArgumentParser(description="LoCoMo benchmark for remem")
    parser.add_argument("--remem-url", default="http://127.0.0.1:5567")
    parser.add_argument("--model", default="gpt-5.4", help="Model for generation + judging")
    parser.add_argument("--top-k", type=int, default=10)
    parser.add_argument("--data-file", type=str, default=str(DATA_PATH))
    parser.add_argument("--skip-ingest", action="store_true")
    parser.add_argument("--max-samples", type=int, default=0, help="0=all")
    parser.add_argument("--sample-index", type=int, default=-1, help="Run single conversation by index (0-9)")
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

    summary = {"config": {"model": args.model, "top_k": args.top_k, "retriever": "remem-fts5-rrf"}}
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
