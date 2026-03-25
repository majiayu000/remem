"""Download LoCoMo dataset from HuggingFace."""

import json
from pathlib import Path

DATA_DIR = Path(__file__).parent / "data"


def download():
    try:
        from huggingface_hub import hf_hub_download
    except ImportError:
        print("Install huggingface_hub: uv pip install huggingface_hub")
        raise SystemExit(1)

    DATA_DIR.mkdir(exist_ok=True)
    out = DATA_DIR / "locomo10.json"
    if out.exists():
        print(f"Already exists: {out}")
        return

    path = hf_hub_download(
        repo_id="snap-research/locomo",
        filename="data/locomo10.json",
        repo_type="dataset",
        local_dir=str(DATA_DIR),
    )
    # hf_hub_download may nest inside data/ subfolder
    downloaded = Path(path)
    if downloaded != out:
        downloaded.rename(out)
    print(f"Downloaded: {out} ({out.stat().st_size / 1024:.0f} KB)")


if __name__ == "__main__":
    download()
