// Bookmarks: places and passages marked in a note or a book's chapter, each with an optional short label and
// one of 15 colours (or none). A coloured square sits at the right of the lines it marks; pressing it shows
// the words it covers, its label, and a way to change it. Kept beside the note, so a reading book's chapters
// can be marked without being changed.
//
// Also the text's right-click menu (Read aloud, Bookmark, and copy and paste), and reading aloud from a place
// clicked in the text.
//
// The text is reached through a "surface", so the note editor and a page that is only read work alike.
// Places in it are character offsets into its text, a line per paragraph:
//   root                         the element the text is in
//   readOnly                     true when the text can't be changed
//   text()                       the text
//   range(from, to)              a DOM Range over it
//   caretRect(at)                where a place is on screen
//   offsetAtPoint(x, y)          the place under a point on screen, or null
//   selection()                  { from, to } selected, or null
//   onChange(f)                  f(map) after an edit; map(at, assoc) gives where a place moved to
//   onReset(f)                   f() when the whole text was replaced
//   onLayout(f)                  f() when the text may have moved on screen
//   paste(text)                  puts text at the cursor (an editor only)

icon.bookmark ??= '<path d="M6 3h12v18l-6-4-6 4z"></path>';
icon.cut = '<circle cx="6" cy="6" r="3"></circle><circle cx="6" cy="18" r="3"></circle><path d="M20 4L8.1 15.9M14.5 14.5L20 20M8.1 8.1L12 12"></path>';
icon.paste = '<path d="M15 2H9a1 1 0 0 0-1 1v2a1 1 0 0 0 1 1h6a1 1 0 0 0 1-1V3a1 1 0 0 0-1-1z"></path><path d="M8 4H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V6a2 2 0 0 0-2-2h-2"></path>';

/// The 15 bookmark colours, as Superteem's colour tags.
const BOOKMARK_COLOURS = [
  ["c01", "#B85C5C", "Red"], ["c02", "#C47A3A", "Orange"], ["c03", "#B8963A", "Gold"], ["c04", "#7A9A3E", "Olive"],
  ["c05", "#3D9468", "Green"], ["c06", "#2E8F86", "Teal"], ["c07", "#3A86B0", "Sky"], ["c08", "#4A6FBE", "Blue"],
  ["c09", "#7460B8", "Violet"], ["c10", "#A45BA0", "Plum"], ["c11", "#B85C80", "Rose"], ["c12", "#8A7364", "Taupe"],
  ["c13", "#6A8494", "Slate"], ["c14", "#5C8A72", "Sage"], ["c15", "#A86B4A", "Copper"],
];
const bookmarkColour = (key) => BOOKMARK_COLOURS.find(([k]) => k === key)?.[1] ?? null;

/// How much of the text either side of a bookmark is kept to find its place again.
const BOOKMARK_CONTEXT = 32;

/// The place under a point on screen, as a DOM node and offset.
function caretAtPoint(x, y) {
  if (document.caretPositionFromPoint) {
    const p = document.caretPositionFromPoint(x, y);
    return p && { node: p.offsetNode, offset: p.offset };
  }
  const r = document.caretRangeFromPoint?.(x, y);
  return r && { node: r.startContainer, offset: r.startOffset };
}

/// A page's text that is only read (a book's chapter) as a surface.
function domSurface(root) {
  const BLOCK = /^(P|H[1-6]|LI|BLOCKQUOTE|PRE|TD|TH|DIV|DT|DD|FIGCAPTION|SECTION|TR)$/;
  let index = null;
  const build = () => {
    let text = "";
    const segs = [];
    let block = null;
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    for (let n = walker.nextNode(); n; n = walker.nextNode()) {
      // Footnote numbers and notes aren't part of the text read aloud or bookmarked.
      if (n.parentElement?.closest("[data-noread]")) continue;
      let b = n.parentElement;
      while (b && b !== root && !BLOCK.test(b.tagName)) b = b.parentElement;
      if (segs.length && b !== block) text += "\n";
      block = b;
      segs.push({ at: text.length, node: n, len: n.data.length });
      text += n.data;
    }
    return { text, segs };
  };
  const idx = () => (index ??= build());
  const locate = (at) => {
    const { segs } = idx();
    if (!segs.length) return { node: root, offset: 0 };
    let lo = 0, hi = segs.length - 1;
    while (lo < hi) { const mid = (lo + hi + 1) >> 1; if (segs[mid].at <= at) lo = mid; else hi = mid - 1; }
    const s = segs[lo];
    return { node: s.node, offset: Math.max(0, Math.min(s.len, at - s.at)) };
  };
  const offsetOf = (node, offset) => {
    const { segs, text } = idx();
    if (node.nodeType === Node.TEXT_NODE) {
      const s = segs.find((seg) => seg.node === node);
      if (s) return s.at + Math.min(offset, s.len);
    }
    // Between elements: the first text at or after the place.
    const r = document.createRange();
    if (node.nodeType === Node.TEXT_NODE) r.setStartAfter(node); else r.setStart(node, offset);
    const s = segs.find((seg) => r.comparePoint(seg.node, 0) >= 0);
    return s ? s.at : text.length;
  };
  const range = (from, to) => {
    const a = locate(from), b = locate(Math.max(from, to));
    const r = document.createRange();
    r.setStart(a.node, a.offset);
    r.setEnd(b.node, b.offset);
    return r;
  };
  return {
    root,
    readOnly: true,
    text: () => idx().text,
    range,
    caretRect(at) {
      const r = range(at, at);
      const rect = [...r.getClientRects()].find((c) => c.height) ?? r.getBoundingClientRect();
      if (rect.height) return rect;
      // Some places have no box of their own: the edge of the character after, or before.
      const len = idx().text.length;
      const next = at < len ? range(at, at + 1).getClientRects()[0] : null;
      if (next) return new DOMRect(next.left, next.top, 0, next.height);
      const prev = at > 0 ? [...range(at - 1, at).getClientRects()].at(-1) : null;
      return prev ? new DOMRect(prev.right, prev.top, 0, prev.height) : root.getBoundingClientRect();
    },
    offsetAtPoint(x, y) {
      const p = caretAtPoint(x, y);
      return p && root.contains(p.node) ? offsetOf(p.node, p.offset) : null;
    },
    selection() {
      const sel = getSelection();
      if (!sel?.rangeCount) return null;
      const r = sel.getRangeAt(0);
      if (!root.contains(r.commonAncestorContainer)) return null;
      return { from: offsetOf(r.startContainer, r.startOffset), to: offsetOf(r.endContainer, r.endOffset) };
    },
    /// Highlights a place, as a search found it, and scrolls it into the middle of the window. The
    /// highlight goes at the next press in the text.
    reveal(from, to) {
      const r = range(from, to);
      if (window.CSS?.highlights && typeof Highlight !== "undefined") {
        CSS.highlights.set("search-hit", new Highlight(r));
        root.addEventListener("pointerdown", () => CSS.highlights.delete("search-hit"), { once: true });
      } else {
        getSelection()?.removeAllRanges();
        getSelection()?.addRange(r);
      }
      const at = r.startContainer.nodeType === Node.TEXT_NODE ? r.startContainer.parentElement : r.startContainer;
      at?.scrollIntoView?.({ block: "center" });
    },
    onChange() {},
    onReset() {},
    onLayout() {},
  };
}

/// Where a bookmark's words are now: those it covered, where the words around them match best and nearest
/// where it was; or, for a place or words no longer there, where the words either side of it meet.
function findBookmark(text, a) {
  const at = Math.min(a.at ?? 0, text.length);
  const prefix = a.prefix ?? "", suffix = a.suffix ?? "";
  const fit = (i, j) => {
    let n = 0;
    while (n < prefix.length && i - n - 1 >= 0 && text[i - n - 1] === prefix[prefix.length - n - 1]) n++;
    let m = 0;
    while (m < suffix.length && j + m < text.length && text[j + m] === suffix[m]) m++;
    return n + m - Math.abs(i - at) / (text.length + 1);
  };
  const best = (needle, place) => {
    if (!needle) return null;
    let found = null;
    for (let i = text.indexOf(needle); i >= 0; i = text.indexOf(needle, i + 1)) {
      const [from, to] = place(i);
      const score = fit(from, to);
      if (!found || score > found.score) found = { from, to, score };
      if (i > 2_000_000) break;
    }
    return found;
  };
  const quote = a.quote ?? "";
  const words = best(quote, (i) => [i, i + quote.length]);
  if (words) return words;
  const point = best(prefix + suffix, (i) => [i + prefix.length, i + prefix.length])
    ?? best(prefix, (i) => [i + prefix.length, i + prefix.length])
    ?? best(suffix, (i) => [i, i]);
  return point ?? { from: at, to: at };
}

/// The right-click menu: `items` are { icon, text, keys, disabled, run }, { heading, sub } (a line saying
/// something, not a choice) or "-".
function showContextMenu(x, y, items) {
  document.querySelector(".context-menu")?.remove();
  const menu = el("div", { class: "context-menu", role: "menu" });
  const close = () => {
    menu.remove();
    document.removeEventListener("pointerdown", outside, true);
    document.removeEventListener("keydown", key, true);
    removeEventListener("blur", close);
    removeEventListener("resize", close);
    document.removeEventListener("scroll", close, true);
  };
  const outside = (e) => { if (!menu.contains(e.target)) close(); };
  const rows = [];
  const key = (e) => {
    if (e.key === "Escape") { e.preventDefault(); close(); return; }
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const live = rows.filter((r) => !r.disabled);
    const now = live.indexOf(document.activeElement);
    live[(now + (e.key === "ArrowDown" ? 1 : -1) + live.length) % live.length]?.focus();
  };
  for (const item of items.filter(Boolean)) {
    if (item === "-") { if (menu.lastChild && menu.lastChild.tagName !== "HR") menu.append(el("hr")); continue; }
    if (item.heading) { menu.append(el("div", { class: "context-heading" }, item.heading, item.sub ? el("small", {}, item.sub) : null)); continue; }
    const row = el("button", { type: "button", role: "menuitem", disabled: !!item.disabled },
      item.icon ? svg(item.icon, 16) : el("span", { class: "context-blank" }), el("span", { class: "grow" }, item.text),
      item.keys ? el("kbd", {}, item.keys) : null);
    // The text keeps its selection and cursor while the menu is used.
    row.addEventListener("mousedown", (e) => e.preventDefault());
    row.addEventListener("click", () => { close(); item.run(); });
    rows.push(row);
    menu.append(row);
  }
  if (menu.lastChild?.tagName === "HR") menu.lastChild.remove();
  document.body.append(menu);
  const box = menu.getBoundingClientRect();
  menu.style.left = `${Math.max(8, Math.min(x, innerWidth - box.width - 8))}px`;
  menu.style.top = `${y + box.height > innerHeight - 8 ? Math.max(8, y - box.height) : y}px`;
  document.addEventListener("pointerdown", outside, true);
  document.addEventListener("keydown", key, true);
  addEventListener("blur", close);
  addEventListener("resize", close);
  document.addEventListener("scroll", close, true);
}

const modKey = () => (platform === "macos" ? "⌘" : "Ctrl+");

/// Bookmarks on one surface. `holder` is an element around the surface's root, which the markers are drawn
/// over. `noteId()` is the note's id, or null before it is saved; `ensureNote()` saves it and gives its id.
/// `onRead(text, { toEnd, from })` reads text aloud, `from` being where in the text it begins. `onVoice()`,
/// for a book with its own voice, changes which voice reads it. `listen`, in a book: { read(from), forget() },
/// for the place reading aloud stopped, which setListen shows. `isShown(mark)` says whether a bookmark is
/// drawn, as a book's choice of colours and makers to show has it (see bookmarkFilter).
/// Returns { create, reading, readingText, reload, redraw, setListen, listenFrom, textFrom }.
function attachBookmarks({ surface, holder, noteId, ensureNote = async () => noteId(), onRead, onVoice, listen, isShown = () => true }) {
  const layer = el("div", { class: "bm-layer", "aria-hidden": "false" });
  holder.classList.add("bm-holder");
  surface.root.classList.add("bm-root");
  holder.append(layer);

  let marks = [];         // { id, from, to, label, color, created, author }
  let shown = null;       // the id of the bookmark whose words and label are showing
  let point = null;       // where the person last clicked in the text, for reading aloud from there
  let loadedFor = null;
  let listened = null;    // where reading aloud last stopped, when it's in this text: { place, from }

  // ----- Keeping them

  const anchorOf = (m, text) => ({
    id: m.id, label: m.label, color: m.color, created: m.created, author: m.author ?? "",
    at: m.from,
    quote: text.slice(m.from, m.to),
    prefix: text.slice(Math.max(0, m.from - BOOKMARK_CONTEXT), m.from),
    suffix: text.slice(m.to, m.to + BOOKMARK_CONTEXT),
  });
  let kept = []; // as last saved or loaded, to find each again when the whole text is replaced
  let saveTimer = null;
  const save = () => {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(async () => {
      const id = noteId();
      if (!id) return;
      const text = surface.text();
      kept = marks.map((m) => anchorOf(m, text));
      await invoke("save_note_bookmarks", { id, bookmarks: kept }).catch((e) => micToast(`Bookmarks not saved: ${e}`));
    }, 500);
  };
  const place = () => {
    const text = surface.text();
    marks = kept.map((a) => {
      const { from, to } = findBookmark(text, a);
      return { id: a.id, label: a.label ?? "", color: a.color ?? "", created: a.created ?? 0, author: a.author ?? "", from, to };
    }).sort((x, y) => x.from - y.from);
    redraw();
  };
  async function reload() {
    const id = noteId();
    loadedFor = id;
    kept = id ? await invoke("note_bookmarks", { id }).catch(() => []) : [];
    if (loadedFor !== id) return;
    place();
  }

  surface.onChange((map) => {
    if (point != null) point = map(point, -1);
    if (listened) listened.from = map(listened.from, 1);
    if (!marks.length) return;
    for (const m of marks) {
      m.from = map(m.from, 1);
      m.to = Math.max(m.from, map(m.to, -1));
    }
    save();
    redraw();
  });
  surface.onReset(() => { point = null; if (listened) setListen(listened.place); place(); });
  surface.onLayout(() => redraw());
  new ResizeObserver(() => redraw()).observe(surface.root);

  // ----- Drawing

  let frame = 0;
  function redraw() {
    if (frame) return;
    frame = requestAnimationFrame(() => { frame = 0; draw(); });
  }

  const rectsOf = (m) => {
    if (m.to > m.from) return [...surface.range(m.from, m.to).getClientRects()].filter((r) => r.width > 0.5 && r.height > 0);
    const r = surface.caretRect(m.from);
    return r && r.height ? [r] : [];
  };

  function draw() {
    if (!layer.isConnected) return;
    layer.replaceChildren();
    const base = layer.getBoundingClientRect();
    const root = surface.root.getBoundingClientRect();
    const pad = parseFloat(getComputedStyle(surface.root).paddingRight) || 0;
    // The squares sit just right of the text: in the page's margin, or in room kept for them on a phone.
    const x = root.right - pad - base.left + (pad >= 22 ? 2 : 8);
    const size = 22;
    let below = -Infinity;
    const seen = marks.filter((m) => isShown(m));
    const all = listened ? [...seen, { id: LISTEN_ID, from: listened.from, to: listened.from, listen: true }].sort((p, q) => p.from - q.from) : seen;
    for (const m of all) {
      const rects = rectsOf(m);
      if (!rects.length) continue;
      const first = rects[0], last = rects.at(-1);
      const top = Math.max(first.top - base.top + (first.height - size) / 2, below + 3);
      below = top + size;
      const open = shown === m.id;
      if (m.listen) {
        const flag = el("button", {
          class: "bm-flag listen" + (open ? " on" : ""), type: "button", style: `left:${x}px;top:${top}px`,
          title: "Where you stopped listening", "aria-label": "Where you stopped listening", "aria-expanded": String(open),
        }, svg("speak", 13));
        flag.addEventListener("mousedown", (e) => e.preventDefault());
        flag.addEventListener("click", () => show(open ? null : m.id));
        layer.append(flag);
        if (!open) continue;
        layer.append(el("div", { class: "bm-point listen", style: `left:${first.left - base.left - 1}px;top:${first.top - base.top}px;height:${first.height}px` }));
        const pop = el("div", { class: "bm-pop", role: "dialog", "aria-label": "Where you stopped listening" },
          el("div", { class: "bm-pop-label" }, "You stopped listening here", el("small", {}, agoText(listened.place.created))),
          el("div", { class: "bm-pop-actions" },
            el("button", { class: "icon-only", type: "button", title: "Read on from here", "aria-label": "Read on from here", onclick: () => { show(null); listen?.read(listened.from); } }, svg("play", 14)),
            el("button", { class: "icon-only", type: "button", title: "Forget this place", "aria-label": "Forget where you stopped listening", onclick: () => { show(null); listen?.forget(); } }, svg("trash", 16))));
        pop.addEventListener("mousedown", (e) => { if (e.target.closest("button")) e.preventDefault(); });
        layer.append(pop);
        const high = pop.offsetHeight;
        pop.style.top = `${top - high - 6 >= -base.top + 4 ? top - high - 6 : top + size + 6}px`;
        pop.style.right = `${base.width - x - size}px`;
        continue;
      }
      const colour = bookmarkColour(m.color);
      const span = last.bottom - first.top > first.height * 1.5;
      if (span) {
        layer.append(el("div", { class: "bm-span" + (colour ? "" : " plain"), style: `left:${x + size / 2 - 1.5}px;top:${top + size}px;height:${Math.max(0, last.bottom - base.top - top - size)}px;${colour ? `--bm:${colour}` : ""}` }));
      }
      const flag = el("button", {
        class: "bm-flag" + (colour ? "" : " plain") + (open ? " on" : ""), type: "button",
        style: `left:${x}px;top:${top}px;${colour ? `--bm:${colour}` : ""}`,
        title: m.label || "Bookmark", "aria-label": m.label ? `Bookmark: ${m.label}` : "Bookmark", "aria-expanded": String(open),
      }, svg("bookmark", 13));
      flag.addEventListener("mousedown", (e) => e.preventDefault());
      flag.addEventListener("click", () => show(open ? null : m.id));
      layer.append(flag);
      if (!open) continue;
      // The words it covers, underlined; or the place it marks.
      const tint = colour ?? "var(--key)";
      if (m.to > m.from) {
        for (const r of rects) {
          layer.append(el("div", { class: "bm-under", style: `left:${r.left - base.left}px;top:${r.top - base.top}px;width:${r.width}px;height:${r.height}px;--bm:${tint}` }));
        }
      } else {
        layer.append(el("div", { class: "bm-point", style: `left:${first.left - base.left - 1}px;top:${first.top - base.top}px;height:${first.height}px;--bm:${tint}` }));
      }
      const pop = el("div", { class: "bm-pop", role: "dialog", "aria-label": "Bookmark", style: `right:${base.width - x + 6}px;top:${top - 4}px` },
        m.label ? el("div", { class: "bm-pop-label" }, m.label, m.author ? el("small", {}, `by ${m.author}`) : null)
          : el("div", { class: "bm-pop-label none" }, m.author ? `No label · by ${m.author}` : "No label"),
        el("div", { class: "bm-pop-actions" },
          el("button", { class: "icon-only", type: "button", title: "Read aloud from here", "aria-label": "Read aloud from here", onclick: () => { show(null); onRead?.(textFrom(m.from, m.to), { toEnd: m.to <= m.from, from: m.to > m.from ? m.from : wordStart(m.from) }); } }, svg("speak", 16)),
          el("button", { class: "icon-only", type: "button", title: "Edit or delete the bookmark", "aria-label": "Edit bookmark", onclick: () => edit(m) }, svg("pen", 16))));
      pop.addEventListener("mousedown", (e) => { if (e.target.closest("button")) e.preventDefault(); });
      layer.append(pop);
      // Above the line, so the words it marks stay in sight; below it at the top of the text.
      const high = pop.offsetHeight;
      pop.style.top = `${top - high - 6 >= -base.top + 4 ? top - high - 6 : top + size + 6}px`;
      pop.style.right = `${base.width - x - size}px`;
    }
    // In text that is only read, the place reading aloud will start from.
    if (surface.readOnly && point != null && !shown) {
      const r = surface.caretRect(point);
      if (r?.height) layer.append(el("div", { class: "bm-readpoint", style: `left:${r.left - base.left - 1}px;top:${r.top - base.top}px;height:${r.height}px` }));
    }
  }

  // The words and label show while a bookmark is open; anything pressed outside closes it.
  const outside = (e) => {
    if (e.target.closest?.(".bm-flag, .bm-pop, dialog")) return;
    show(null);
  };
  const escape = (e) => { if (e.key === "Escape") show(null); };
  function show(id) {
    shown = id;
    document.removeEventListener("pointerdown", outside, true);
    document.removeEventListener("keydown", escape);
    if (id) {
      document.addEventListener("pointerdown", outside, true);
      document.addEventListener("keydown", escape);
    }
    redraw();
  }

  // ----- Making and changing them

  async function create(range) {
    const sel = range ?? surface.selection() ?? (point != null ? { from: point, to: point } : null);
    if (!sel) { micToast("Click in the text, or select some, to bookmark it."); return; }
    const text = surface.text();
    let { from, to } = sel;
    if (to < from) [from, to] = [to, from];
    // Spaces at either end of what was selected aren't part of it.
    while (to > from && /\s/.test(text[to - 1])) to--;
    while (from < to && /\s/.test(text[from])) from++;
    const fresh = { id: `b${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`, from, to, label: "", color: "", author: "", created: Math.floor(Date.now() / 1000) };
    const result = await bookmarkDialog(fresh, text.slice(from, to), true);
    if (!result) return;
    try {
      if (!noteId()) { await ensureNote(); loadedFor = noteId(); }
    } catch (e) {
      micToast(String(e));
      return;
    }
    // The text may have moved while the dialog was open, as a dictation wrote in.
    marks.push({ ...fresh, ...result });
    marks.sort((x, y) => x.from - y.from);
    save();
    show(fresh.id);
    if (!isShown({ ...fresh, ...result })) micToast("Added, but bookmarks like it are hidden. Hold the Bookmark button to show them.");
  }

  async function edit(m) {
    const result = await bookmarkDialog(m, surface.text().slice(m.from, m.to), false);
    if (!result) return;
    if (result.remove) {
      marks = marks.filter((x) => x !== m);
      show(null);
    } else {
      Object.assign(m, result);
      show(m.id);
    }
    save();
  }

  // ----- Reading aloud from a place

  /// The start of the word at `from`.
  function wordStart(from) {
    const text = surface.text();
    let start = Math.min(from, text.length);
    while (start > 0 && /[\p{L}\p{N}'’-]/u.test(text[start - 1])) start--;
    return start;
  }

  /// From the start of the word at `from`, to `to` or the end.
  function textFrom(from, to = from) {
    const text = surface.text();
    if (to > from) return text.slice(from, to);
    return text.slice(wordStart(from));
  }

  /// What reading aloud reads, and where in the text it begins: what is selected, else from the place last
  /// clicked; null for all of it.
  function reading() {
    const sel = surface.selection();
    if (sel && sel.to !== sel.from) {
      const from = Math.min(sel.from, sel.to);
      return { text: textFrom(from, Math.max(sel.from, sel.to)), from };
    }
    if (point == null) return null;
    const rest = textFrom(point, point);
    return rest.trim() && rest.length < surface.text().length ? { text: rest, from: wordStart(point) } : null;
  }
  const readingText = () => reading()?.text ?? null;

  /// Shows where reading aloud last stopped, when that's in this text (`place` as books keep it); null for none.
  function setListen(place) {
    listened = place ? { place, from: findBookmark(surface.text(), place).from } : null;
    if (!listened && shown === LISTEN_ID) shown = null;
    redraw();
  }
  /// Where reading aloud stopped in this text, or null; null too at its very end, where nothing is left to read.
  function listenFrom() {
    if (!listened) return null;
    return surface.text().slice(listened.from).trim() ? listened.from : null;
  }

  // A click in the text is where reading aloud goes on from.
  surface.root.addEventListener("click", (e) => {
    // The place under the pointer: an editor's own selection may not have caught up yet.
    if (e.button !== 0 || getSelection()?.isCollapsed === false) return;
    const at = surface.offsetAtPoint(e.clientX, e.clientY);
    if (at == null) return;
    point = at;
    if (surface.readOnly) redraw();
  });

  // ----- The right-click menu

  surface.root.addEventListener("contextmenu", (e) => {
    // A phone's long press selects words, with its own menu.
    if (matchMedia("(pointer: coarse)").matches && !matchMedia("(pointer: fine)").matches) return;
    if (e.target.closest?.("a[href], img")) return;
    e.preventDefault();
    const sel = surface.selection();
    const selected = sel && sel.from !== sel.to ? { from: Math.min(sel.from, sel.to), to: Math.max(sel.from, sel.to) } : null;
    const at = selected ? null : surface.offsetAtPoint(e.clientX, e.clientY) ?? sel?.from ?? null;
    if (!selected && at != null) { point = at; if (surface.readOnly) redraw(); }
    const here = selected ?? (at != null ? { from: at, to: at } : null);
    const mod = modKey();
    showContextMenu(e.clientX, e.clientY, [
      { icon: "speak", text: selected ? "Read aloud" : "Read aloud from here", disabled: !here, run: () => onRead?.(textFrom(here.from, here.to), { toEnd: !selected, from: selected ? here.from : wordStart(here.from) }) },
      onVoice && { icon: "speak", text: "Voice for this book…", run: () => onVoice(here) },
      { icon: "bookmark", text: selected ? "Bookmark…" : "Bookmark here…", disabled: !here, run: () => create(here) },
      "-",
      !surface.readOnly && { icon: "cut", text: "Cut", keys: `${mod}X`, disabled: !selected, run: () => document.execCommand("cut") },
      { icon: "copy", text: "Copy", keys: `${mod}C`, disabled: !selected, run: () => document.execCommand("copy") },
      !surface.readOnly && surface.paste && { icon: "paste", text: "Paste", keys: `${mod}V`, run: async () => {
        try { surface.paste(await navigator.clipboard.readText()); } catch { micToast(`Paste with ${mod}V.`); }
      } },
    ]);
  });

  reload();
  return { create, reading, readingText, reload, redraw, setListen, listenFrom, textFrom };
}

const LISTEN_ID = "listened";

/// "just now", "5 minutes ago", "yesterday", "3 days ago", or the date.
function agoText(seconds) {
  const s = Math.max(0, Date.now() / 1000 - (seconds || 0));
  if (s < 90) return "just now";
  if (s < 3600) return `${Math.round(s / 60)} minutes ago`;
  if (s < 86400) { const h = Math.round(s / 3600); return `${h} hour${h === 1 ? "" : "s"} ago`; }
  if (s < 2 * 86400) return "yesterday";
  if (s < 14 * 86400) return `${Math.floor(s / 86400)} days ago`;
  return new Date(seconds * 1000).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
}

// ----- Where reading aloud stopped in a book

/// Stopping reading aloud after hearing more than this many words keeps the place; fewer, and it's taken as
/// a try or a slip, and the place stays where it was.
const LISTEN_MIN_WORDS = 20;

/// A book's place where reading aloud last stopped: one per book, kept beside it. `get()` is it or null
/// ({ note, at, quote, prefix, suffix, created }); `watch(f)` is told now and each time it changes.
function listenPlace(bookId) {
  let place = null;
  const watchers = new Set();
  const tell = () => { for (const f of watchers) f(place); };
  const ready = invoke("book_listen_place", { id: bookId }).then((p) => { place = p ?? null; tell(); }).catch(() => {});
  return {
    ready,
    get: () => place,
    watch(f) { watchers.add(f); f(place); },
    async set(next) {
      place = next;
      tell();
      try {
        await invoke("set_book_listen_place", { id: bookId, place: next });
      } catch (e) {
        micToast(`Where you stopped listening wasn't kept: ${e}`);
      }
    },
    forget() { return this.set(null); },
  };
}

/// Where a reading that ended stopped, in a book's chapters: `texts` is each chapter's text in order, and
/// `reading.start` is { chapter, at }, where in them the reading began. Follows the pieces said through the
/// text to the one it would go on from. Gives { chapter, at, quote, end } (`end` when it read to its end), or
/// null when too little was heard to count.
function stoppedAt(texts, reading) {
  const { pieces, start } = reading;
  if (!start || !texts.length || !pieces?.length || reading.heard <= LISTEN_MIN_WORDS) return null;
  const end = reading.at >= pieces.length;
  const target = end ? pieces.length - 1 : reading.at;
  let c = Math.max(0, Math.min(start.chapter ?? 0, texts.length - 1)), at = start.at ?? 0;
  let found = { chapter: c, from: at, to: at };
  for (let i = 0; i <= target; i++) {
    const want = pieces[i].text;
    let hit = null;
    // In this chapter after the last piece, or else from the top of the next.
    for (let k = c; k < Math.min(texts.length, c + 2) && !hit; k++) {
      const j = texts[k].indexOf(want, k === c ? at : 0);
      if (j >= 0) hit = { chapter: k, from: j, to: j + want.length };
    }
    if (hit) found = hit;
    // A chapter's title isn't in its text: once a chapter has been read into, a title is the next one's.
    else if (pieces[i].before >= PAUSE.heading && at > 0 && c + 1 < texts.length) found = { chapter: c + 1, from: 0, to: 0 };
    else found = { chapter: c, from: at, to: at };
    c = found.chapter;
    at = found.to;
  }
  return { chapter: found.chapter, at: end ? found.to : found.from, quote: end ? "" : pieces[target].text, end };
}

/// A place in a chapter's text, as a book keeps it: found again by the words either side, as a bookmark is.
function listenAnchor(note, text, at, quote = "") {
  const q = text.slice(at, at + quote.length) === quote ? quote : "";
  return {
    note, at, quote: q.slice(0, 400),
    prefix: text.slice(Math.max(0, at - BOOKMARK_CONTEXT), at),
    suffix: text.slice(at + q.length, at + q.length + BOOKMARK_CONTEXT),
    created: Math.floor(Date.now() / 1000),
  };
}

/// Says, the first few times, that the place was kept and how to get back to it.
function toldListenPlace(moved) {
  let told = 0;
  try { told = Number(localStorage.getItem("listen-place-told")) || 0; } catch {}
  if (told >= 3) return;
  try { localStorage.setItem("listen-place-told", String(told + 1)); } catch {}
  micToast(`${moved ? "Moved" : "Kept"} where you stopped listening. Read aloud goes on from there; hold it for more.`);
}

/// Asks for a bookmark's label and colour. Gives { label, color }, { remove: true }, or null.
function bookmarkDialog(m, quote, fresh) {
  return new Promise((done) => {
    let result = null;
    let colour = m.color ?? "";
    const label = el("input", { type: "text", value: m.label ?? "", placeholder: "A few words, or leave it empty", maxlength: "120", "aria-label": "Label" });
    const swatches = el("div", { class: "bm-swatches", role: "radiogroup", "aria-label": "Colour" });
    const choose = (key) => {
      colour = key;
      for (const s of swatches.children) {
        const on = s.dataset.key === key;
        s.classList.toggle("on", on);
        s.setAttribute("aria-checked", String(on));
      }
    };
    for (const [key, hex, name] of [["", null, "No colour"], ...BOOKMARK_COLOURS]) {
      const s = el("button", { type: "button", role: "radio", class: "bm-swatch" + (hex ? "" : " none"), style: hex ? `--bm:${hex}` : "", title: name, "aria-label": name });
      s.dataset.key = key;
      s.addEventListener("click", () => choose(key));
      swatches.append(s);
    }
    choose(colour);
    const finish = (value) => { result = value; dialog.close(); };
    const shortQuote = quote.length > 180 ? `${quote.slice(0, 177).trimEnd()}…` : quote;
    const dialog = el("dialog", { class: "sheet bm-dialog", "aria-label": fresh ? "Add bookmark" : "Edit bookmark" },
      el("h2", {}, fresh ? "Add bookmark" : "Edit bookmark"),
      shortQuote.trim() ? el("blockquote", { class: "bm-quote" }, shortQuote) : el("p", { class: "hint bm-quote-none" }, "Marks a place in the text."),
      el("label", { class: "field" }, "Label", label),
      el("div", { class: "field" }, "Colour", swatches),
      el("div", { class: "actions" },
        fresh ? null : el("button", { class: "link danger", type: "button", onclick: () => finish({ remove: true }) }, "Delete"),
        el("span", { class: "grow" }),
        el("button", { class: "pill", type: "button", onclick: () => dialog.close() }, "Cancel"),
        el("button", { class: "primary pill", type: "button", onclick: () => finish({ label: label.value.trim(), color: colour }) }, fresh ? "Add" : "Save")));
    label.addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); finish({ label: label.value.trim(), color: colour }); } });
    dialog.addEventListener("close", () => { dialog.remove(); done(result); });
    document.body.append(dialog);
    dialog.showModal();
    label.focus();
    label.select();
  });
}

// ----- A book's bookmarks, all at once

/// Runs `open` when `button` is held down for half a second, or right-clicked, instead of its click.
function onHold(button, open) {
  let held = false;
  let timer = null;
  button.addEventListener("pointerdown", (e) => {
    held = false;
    clearTimeout(timer);
    if (e.button === 0) timer = setTimeout(() => { held = true; open(); }, 500);
  });
  for (const type of ["pointerup", "pointerleave", "pointercancel"]) button.addEventListener(type, () => clearTimeout(timer));
  button.addEventListener("contextmenu", (e) => { e.preventDefault(); clearTimeout(timer); if (!held) open(); });
  // The click that ends a hold isn't a press.
  button.addEventListener("click", (e) => { if (held) { held = false; e.preventDefault(); e.stopImmediatePropagation(); } }, true);
}

/// Which of a book's bookmarks are shown, by colour and by who made them: { view, isShown(mark), load(), set(view) }.
/// `isShown` can be given to attachBookmarks before the choice has loaded; everything shows until it has.
function bookmarkFilter(bookId) {
  const f = {
    view: { hidden_colours: [], hidden_authors: [] },
    isShown: (m) => !f.view.hidden_colours.includes(m.color ?? "") && !f.view.hidden_authors.includes(m.author ?? ""),
    async load() { f.view = await invoke("book_bookmark_view", { id: bookId }).catch(() => f.view); return f.view; },
    async set(view) {
      f.view = view;
      await invoke("set_book_bookmark_view", { id: bookId, view }).catch((e) => micToast(`Not kept: ${e}`));
    },
  };
  return f;
}

/// The name a person's own bookmarks go out under, remembered on this device.
const bookmarkAuthor = {
  get() { try { return localStorage.getItem("bookmark-author") ?? ""; } catch { return ""; } },
  set(name) { try { localStorage.setItem("bookmark-author", name); } catch {} },
};

/// All the bookmarks in a book, opened by holding the Bookmark button: show or hide them by colour and by
/// who made them, export them all to give to someone, import someone's, and delete an import. `filter` is the
/// book's bookmarkFilter; `onChange("view")` after the choice of what shows changes, `onChange("marks")` after
/// bookmarks were imported or deleted.
async function bookBookmarksDialog(book, filter, onChange) {
  const plural = (n, one, many = `${one}s`) => `${n.toLocaleString()} ${n === 1 ? one : many}`;
  const were = (n, rest) => `${n.toLocaleString()} ${n === 1 ? "was" : "were"} ${rest}`;
  const title = book.title || "this book";
  const dialog = el("dialog", { class: "sheet bm-book-dialog", "aria-label": `Bookmarks in ${title}` });
  const body = el("div", { class: "bm-book-body" });
  const status = el("p", { class: "hint bm-book-status", "aria-live": "polite" });
  dialog.append(el("div", { class: "bm-book-head" }, el("h2", {}, `Bookmarks in ${title}`),
    el("button", { class: "icon-only", type: "button", title: "Close", "aria-label": "Close", onclick: () => dialog.close() }, svg("close", 16))),
    body, status);
  dialog.addEventListener("close", () => dialog.remove());
  document.body.append(dialog);
  dialog.showModal();

  const say = (...parts) => status.replaceChildren(...parts);

  async function draw() {
    const [byNote] = await Promise.all([invoke("book_bookmarks", { id: book.id }).catch(() => ({})), filter.load()]);
    const all = Object.values(byNote).flat();
    const chapters = Object.keys(byNote).length;
    const count = (key, value) => all.filter((m) => (m[key] ?? "") === value).length;
    const colours = [...new Set(all.map((m) => m.color ?? ""))].sort((a, b) => (a || "z").localeCompare(b || "z"));
    const authors = [...new Set(all.map((m) => m.author ?? ""))].sort((a, b) => (a === "" ? -1 : b === "" ? 1 : a.localeCompare(b)));
    const view = filter.view;
    const hiddenCount = all.filter((m) => !filter.isShown(m)).length;

    const toggle = async (list, value) => {
      const next = { ...view, [list]: view[list].includes(value) ? view[list].filter((v) => v !== value) : [...view[list], value] };
      await filter.set(next);
      onChange?.("view");
      draw();
    };
    const chip = (on, label, n, list, value, swatch) => el("button", {
      class: "bm-chip" + (on ? " on" : ""), type: "button", "aria-pressed": String(on),
      title: on ? `Showing. Press to hide ${label.toLowerCase()} bookmarks.` : `Hidden. Press to show ${label.toLowerCase()} bookmarks.`,
      onclick: () => toggle(list, value),
    }, swatch ?? null, el("span", {}, label), el("small", {}, String(n)));

    const showing = el("section", { class: "bm-book-section" },
      el("h3", {}, "Show"),
      all.length ? el("p", { class: "hint" }, `${plural(all.length, "bookmark")} in ${plural(chapters, "chapter")}.`
        + (hiddenCount ? ` ${hiddenCount.toLocaleString()} ${hiddenCount === 1 ? "is" : "are"} hidden.` : " All are showing."))
        : el("p", { class: "hint" }, "No bookmarks in this book yet. Select some words or click in the text, then press Bookmark."),
      colours.length ? el("div", { class: "bm-chip-row", role: "group", "aria-label": "By colour" },
        el("span", { class: "bm-chip-label" }, "By colour"),
        ...colours.map((c) => {
          const name = BOOKMARK_COLOURS.find(([k]) => k === c)?.[2] ?? "No colour";
          const hex = bookmarkColour(c);
          return chip(!view.hidden_colours.includes(c), name, count("color", c), "hidden_colours", c,
            el("span", { class: "bm-chip-swatch" + (hex ? "" : " none"), style: hex ? `--bm:${hex}` : "" }));
        })) : null,
      authors.length ? el("div", { class: "bm-chip-row", role: "group", "aria-label": "By who made them" },
        el("span", { class: "bm-chip-label" }, "By who made them"),
        ...authors.map((a) => {
          const row = el("span", { class: "bm-chip-pair" }, chip(!view.hidden_authors.includes(a), a || "You", count("author", a), "hidden_authors", a));
          if (a) {
            const remove = el("button", { class: "icon-only bm-chip-remove", type: "button", title: `Delete ${a}'s bookmarks`, "aria-label": `Delete ${a}'s bookmarks` }, svg("trash", 14));
            remove.addEventListener("click", async () => {
              if (!remove.dataset.armed) {
                remove.dataset.armed = "1";
                remove.classList.add("armed");
                say(`Press the bin again to delete all ${plural(count("author", a), "bookmark")} by ${a}. They can be imported again from their file.`);
                setTimeout(() => { delete remove.dataset.armed; remove.classList.remove("armed"); }, 4000);
                return;
              }
              try {
                const n = await invoke("remove_book_bookmarks_by", { id: book.id, author: a });
                say(`Deleted ${plural(n, "bookmark")} by ${a}.`);
                onChange?.("marks");
                draw();
              } catch (e) { say(el("span", { class: "danger" }, String(e))); }
            });
            row.append(remove);
          }
          return row;
        })) : null,
      hiddenCount ? el("button", { class: "link", type: "button", onclick: async () => {
        await filter.set({ hidden_colours: [], hidden_authors: [] });
        onChange?.("view");
        draw();
      } }, "Show them all") : null);

    // Export: every bookmark in the book, the person's own under the name they give.
    const name = el("input", { type: "text", value: bookmarkAuthor.get(), placeholder: "Your name", maxlength: "80", "aria-label": "Your name, as others will see it" });
    const exportButton = el("button", { class: "pill with-icon", type: "button", disabled: !all.length }, svg("export", 14), all.length ? `Export all ${plural(all.length, "bookmark")}` : "Export");
    exportButton.addEventListener("click", async () => {
      const who = name.value.trim();
      if (!who && authors.includes("")) { name.focus(); return say("Add your name first, so people who import them know whose they are."); }
      bookmarkAuthor.set(who);
      exportButton.disabled = true;
      try {
        const path = await invoke("export_book_bookmarks", { id: book.id, author: who });
        say(`Saved to ${path}. `, el("button", { class: "link inline", type: "button", onclick: () => invoke("reveal_export", { path }).catch((e) => say(String(e))) }, "Show in folder"));
      } catch (e) {
        say(el("span", { class: "danger" }, String(e)));
      }
      exportButton.disabled = false;
    });
    const exporting = el("section", { class: "bm-book-section" },
      el("h3", {}, "Give them to someone"),
      el("p", { class: "hint" }, "Saves every bookmark in the book to one file. Anyone with the book can import it."),
      el("label", { class: "field" }, "Your name, as others will see it", name),
      exportButton);

    // Import: a file someone gave, previewed before anything is added.
    const picker = el("input", { type: "file", accept: ".json,application/json", hidden: true });
    const preview = el("div", { class: "bm-import-preview" });
    const importButton = el("button", { class: "pill with-icon", type: "button" }, svg("import", 14), "Import bookmarks…");
    importButton.addEventListener("click", () => picker.click());
    picker.addEventListener("change", async () => {
      const chosen = picker.files?.[0];
      picker.value = "";
      if (!chosen) return;
      let file;
      try {
        file = JSON.parse(await chosen.text());
      } catch {
        return preview.replaceChildren(el("p", { class: "danger" }, "That file can't be read as bookmarks."));
      }
      if (file?.kind !== "superteem-bookmarks" || typeof file.notes !== "object") {
        return preview.replaceChildren(el("p", { class: "danger" }, "That isn't a file of bookmarks exported from Superteem."));
      }
      const incoming = Object.entries(file.notes).flatMap(([note, list]) => (Array.isArray(list) ? list.map((m) => ({ ...m, note })) : []));
      const makers = [...new Set(incoming.map((m) => m.author || "someone"))];
      const shared = incoming.filter((m) => book.notes?.includes(m.note)).length;
      const go = el("button", { class: "primary pill", type: "button", disabled: !shared }, shared ? `Import ${plural(shared, "bookmark")}` : "Nothing to import");
      go.addEventListener("click", async () => {
        go.disabled = true;
        try {
          const r = await invoke("import_book_bookmarks", { id: book.id, file, author: "" });
          preview.replaceChildren();
          say([`Imported ${plural(r.added, "bookmark")}.`, r.already ? `${were(r.already, "already here")}.` : "",
            r.elsewhere ? `${were(r.elsewhere, "for chapters not in this book")}.` : ""].filter(Boolean).join(" "));
          onChange?.("marks");
          draw();
        } catch (e) {
          go.disabled = false;
          say(el("span", { class: "danger" }, String(e)));
        }
      });
      preview.replaceChildren(
        el("p", {}, `${plural(incoming.length, "bookmark")} by ${makers.join(", ")}${file.title ? ` for “${file.title}”` : ""}.`),
        file.book && file.book !== book.id ? el("p", { class: "hint" }, `These were made for another book. Only those in chapters this book has (${shared.toLocaleString()}) can come in.`) : null,
        file.version && book.version && file.version !== book.version
          ? el("p", { class: "hint" }, `They were made in version ${file.version}; this is version ${book.version}. Where the text changed, a few may land near their place rather than on it.`) : null,
        el("div", { class: "actions" }, el("span", { class: "grow" }),
          el("button", { class: "pill", type: "button", onclick: () => preview.replaceChildren() }, "Cancel"), go));
    });
    const importing = el("section", { class: "bm-book-section" },
      el("h3", {}, "Take in someone's"),
      el("p", { class: "hint" }, "Adds their bookmarks beside yours, marked with their name. Yours stay as they are."),
      importButton, picker, preview);

    body.replaceChildren(showing, exporting, importing);
  }
  await draw();
}
