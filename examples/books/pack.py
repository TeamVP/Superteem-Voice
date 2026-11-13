"""Turn examples/books/*.md into the note and book JSON the app ships."""

import json
import re
from pathlib import Path

HERE = Path(__file__).resolve().parent
OUT = HERE.parents[1] / "desktop" / "resources" / "example-books"
NOTES = OUT / "notes"
STAMP = 1763030400
SMALL = {"a", "an", "the", "of", "and", "with"}


def readable(slug: str) -> str:
    words = slug.split("_")
    out = []
    for i, word in enumerate(words):
        out.append(word if i and word in SMALL else word[:1].upper() + word[1:])
    return " ".join(out)


def split_md(text: str) -> tuple[str, str]:
    title = ""
    body = text
    if text.startswith("---\n"):
        end = text.find("\n---\n", 4)
        if end != -1:
            head, body = text[4:end], text[end + 5 :]
            match = re.search(r'^title:\s*("(?:\\.|[^"])*")\s*$', head, re.M)
            if match:
                title = json.loads(match.group(1))
    lines = body.splitlines()
    while lines and not lines[0].strip():
        lines.pop(0)
    if lines and lines[0].startswith("# "):
        lines.pop(0)
    while lines and not lines[0].strip():
        lines.pop(0)
    return title, ("\n".join(lines).rstrip() + "\n")


def main() -> None:
    NOTES.mkdir(parents=True, exist_ok=True)
    for old in NOTES.glob("*.json"):
        old.unlink()
    parts: dict[str, dict] = {}
    order: list[str] = []
    for path in sorted(HERE.glob("*.md")):
        stem = path.stem
        numbered = re.fullmatch(r"(\d{2})_(\d{2})_(.+)", stem)
        if numbered:
            book_n, chapter_n, slug = numbered.groups()
            part = f"Book {int(book_n)}"
            label = f"Chapter {int(chapter_n)}"
            note_id = f"plutarch-{book_n}-{chapter_n}-{slug.replace('_', '-')}"
            title, body = split_md(path.read_text(encoding="utf-8"))
            title = title or readable(slug)
        elif re.fullmatch(r"00_.+", stem):
            part, label = "Preface", ""
            note_id = "plutarch-00-preface"
            _, body = split_md(path.read_text(encoding="utf-8"))
            title = "Preface"
        else:
            raise SystemExit(f"unrecognised name: {path.name}")
        if not re.fullmatch(r"[A-Za-z0-9-]+", note_id):
            raise SystemExit(f"bad id {note_id}")
        if "PROJECT GUTENBERG" in body:
            raise SystemExit(f"license text in {path.name}")
        note = {
            "id": note_id,
            "title": title,
            "body": body,
            "created": STAMP,
            "updated": STAMP,
            "format": "md",
        }
        (NOTES / f"{note_id}.json").write_text(json.dumps(note, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
        if part not in parts:
            parts[part] = {"title": part, "chapters": []}
            order.append(part)
        parts[part]["chapters"].append({"note": note_id, "label": label})

    book_parts = [parts[name] for name in order]
    notes = [chapter["note"] for part in book_parts for chapter in part["chapters"]]
    book = {
        "id": "plutarch-lives",
        "title": "Plutarch's Lives",
        "created": STAMP,
        "updated": STAMP,
        "notes": notes,
        "parts": book_parts,
        "editable": False,
    }
    (OUT / "plutarch-lives.json").write_text(json.dumps(book, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    print(f"wrote {len(notes)} notes in {len(book_parts)} parts")


if __name__ == "__main__":
    main()
