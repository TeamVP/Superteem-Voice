"use strict";

/// A note's chat with the analysis model, in a panel to its right on a computer, and the note's versions:
/// every rewrite the assistant makes is a new version, and "Versions", beside Translate, opens them to
/// read, search and bring back.

icon.history = '<path d="M3 12a9 9 0 1 0 3-6.7L3 8"></path><path d="M3 3v5h5"></path><path d="M12 7v5l3 2"></path>';
icon.chat = '<path d="M21 12a8 8 0 0 1-11.6 7.1L4 20l1-4.6A8 8 0 1 1 21 12z"></path>';
icon.send = '<path d="M5 12h14M13 6l6 6-6 6"></path>';
icon.search = '<circle cx="11" cy="11" r="7"></circle><path d="M20 20l-4-4"></path>';

// ---------- Chat

/// The chat on the page now: { draw(), id() }.
let noteChatNow = null;
/// What the model is doing for the chat on the page: "Loading the model…" until it starts thinking.
let noteChatStep = null;

const CHAT_IDEAS = ["Summarise this note", "Fix spelling and grammar", "Make it shorter", "Turn it into bullet points"];

listen("analysing", (e) => {
  if (e.payload.template !== "chat" || e.payload.id !== note?.id) return;
  noteChatStep = "Thinking…";
  noteChatNow?.draw();
});
listen("analysed", (e) => {
  if (e.payload.template === "chat" && e.payload.kind === "note") noteChatAnswered(e.payload.id);
});

/// The chat beside a note: a button for the top bar and the panel. `status` is the note's "Saved" line.
function noteChat(status) {
  const toggle = el("button", { class: "pill chat-toggle", type: "button", title: "Ask about this note, or have it rewritten", "aria-pressed": "false" },
    svg("chat", 16), el("span", { class: "label" }, "Chat"));
  const log = el("div", { class: "chat-log", role: "log", "aria-live": "polite" });
  const input = el("textarea", { class: "chat-input", rows: "1", placeholder: "Ask about this note, or ask for a rewrite", "aria-label": "Message" });
  const send = el("button", { class: "primary icon-only chat-send", type: "submit", title: "Send (Enter)", "aria-label": "Send" }, svg("send", 16));
  const form = el("form", { class: "chat-form" }, input, send);
  const clear = el("button", { class: "link", type: "button", hidden: true }, "Clear");
  const close = el("button", { class: "icon-only", type: "button", title: "Close the chat", "aria-label": "Close the chat" }, svg("close", 16));
  const panel = el("aside", { class: "note-chat", "aria-label": "Chat about this note", hidden: true },
    el("div", { class: "chat-head" }, el("strong", {}, "Chat"), el("span", { class: "hint" }, "On this device"), el("span", { class: "grow" }), clear, close),
    log, form);
  const mine = note;
  const chat = { messages: [], ready: true, running: false, loaded: false, notices: new Map() };
  mine.chat = chat;

  const isOpen = () => !panel.hidden;
  const setOpen = (open, remember = true) => {
    panel.hidden = !open;
    toggle.setAttribute("aria-pressed", String(open));
    toggle.classList.toggle("on", open);
    document.body.classList.toggle("note-chat-open", open);
    if (remember) localStorage.setItem("note-chat", open ? "1" : "");
    if (open) { load(); if (remember) input.focus(); }
  };
  toggle.addEventListener("click", () => setOpen(!isOpen()));
  close.addEventListener("click", () => setOpen(false));

  const load = async () => {
    if (note !== mine) return;
    if (!mine.id) { chat.loaded = true; return draw(); }
    try {
      const data = await invoke("get_note_chat", { id: mine.id });
      if (note !== mine) return;
      Object.assign(chat, data, { loaded: true });
      if (!chat.running) noteChatStep = null;
    } catch (e) {
      chat.error = String(e);
    }
    draw();
  };
  chat.load = load;

  const ask = async (text) => {
    text = text.trim();
    if (!text || chat.running || note !== mine) return;
    chat.error = null;
    try {
      // The model reads the note as saved, so what's on screen is saved first.
      const id = await mine.save();
      chat.sent = mine.editorText();
      noteChatStep = null;
      chat.running = true;
      chat.messages.push({ role: "user", text, at: Date.now() / 1000 });
      input.value = "";
      grow();
      draw();
      await invoke("chat_note", { id, text });
    } catch (e) {
      chat.error = String(e);
      chat.running = false;
      await load();
    }
  };
  form.addEventListener("submit", (e) => { e.preventDefault(); ask(input.value); });
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) { e.preventDefault(); ask(input.value); }
    if (e.key === "Escape") setOpen(false);
  });
  const grow = () => { input.style.height = "auto"; input.style.height = `${Math.min(input.scrollHeight + 2, 160)}px`; };
  input.addEventListener("input", grow);
  clear.addEventListener("click", async () => {
    if (!mine.id || chat.running) return;
    await invoke("clear_note_chat", { id: mine.id }).catch(() => {});
    chat.notices.clear();
    load();
  });

  const bubble = (m, last) => {
    if (m.role === "user") return el("div", { class: "chat-msg mine" }, m.text);
    if (m.error) return el("div", { class: "chat-msg theirs failed" }, m.error);
    const actions = el("div", { class: "chat-actions" });
    const box = el("div", { class: "chat-msg theirs" }, el("div", { class: "chat-text" }, m.text), actions);
    if (m.version) {
      box.classList.add("rewrote");
      box.prepend(el("div", { class: "chat-made with-icon" }, svg("history", 14), `Rewrote the note · Version ${m.version}`));
      const notice = chat.notices.get(m.version);
      if (notice) box.append(el("div", { class: "hint" }, notice));
      if (last && !notice) actions.append(el("button", { class: "link", type: "button", onclick: () => undoRewrite(m.version, status) }, "Undo"));
      actions.append(el("button", { class: "link", type: "button", onclick: () => showNoteVersions(status, m.version) }, "See versions"));
    } else {
      const copy = el("button", { class: "link", type: "button" }, "Copy");
      copy.addEventListener("click", async () => {
        await navigator.clipboard.writeText(m.text).catch(() => {});
        copy.textContent = "Copied";
        setTimeout(() => { copy.textContent = "Copy"; }, 1500);
      });
      const add = el("button", { class: "link", type: "button", title: "Put this at the end of the note" }, "Add to note");
      add.addEventListener("click", () => { if (note === mine) { mine.addText(m.text); add.textContent = "Added"; add.disabled = true; } });
      actions.append(copy, add);
    }
    return box;
  };

  const draw = () => {
    if (note !== mine || !isOpen()) return;
    const rows = [];
    if (!chat.ready) {
      rows.push(el("div", { class: "chat-empty" },
        el("p", {}, "Chat runs on the same on-device model as analysis templates. Nothing leaves this computer."),
        el("button", { class: "primary pill", type: "button", onclick: async () => {
          const id = await mine.save().catch(() => null);
          analysisOrigin = id ? { kind: "note", id } : null;
          showAnalysisSettings(null, true);
        } }, "Set up the model")));
    } else if (!chat.messages.length && chat.loaded) {
      rows.push(el("div", { class: "chat-empty" },
        el("p", {}, "Ask anything about this note, or ask for it to be rewritten."),
        el("p", { class: "hint" }, "A rewrite becomes a new version of the note, so the one you have now is never lost."),
        el("div", { class: "chat-ideas" }, CHAT_IDEAS.map((idea) => el("button", { class: "pill", type: "button", onclick: () => ask(idea) }, idea)))));
    }
    chat.messages.forEach((m, i) => rows.push(bubble(m, i === chat.messages.length - 1)));
    if (chat.running) {
      rows.push(el("div", { class: "chat-msg theirs pending" }, el("span", { class: "spinner" }), noteChatStep ?? (recording ? "Waiting until the recording stops…" : "Getting the model ready…"),
        el("button", { class: "link", type: "button", onclick: () => invoke("cancel_analysis", { kind: "note", id: mine.id, template: "chat" }).then(() => setTimeout(load, 300)) }, "Stop")));
    }
    if (chat.error) rows.push(el("div", { class: "chat-msg theirs failed" }, chat.error));
    log.replaceChildren(...rows);
    log.scrollTop = log.scrollHeight;
    clear.hidden = !chat.messages.length || chat.running;
    input.disabled = !chat.ready;
    send.disabled = !chat.ready || chat.running;
  };
  chat.draw = draw;
  noteChatNow = { draw, id: () => mine.id };

  /// Opens the panel if it was left open; called once the page is in place.
  const sync = () => setOpen(localStorage.getItem("note-chat") === "1", false);
  return { toggle, panel, sync };
}

/// An answer arrived for a note's chat. When it rewrote the note that's open, the new text goes in the
/// editor, unless the person typed meanwhile: then their text stays and the rewrite waits in Versions.
async function noteChatAnswered(id) {
  const mine = note;
  if (!mine || mine.id !== id || view.kind !== "note" || !mine.chat) return;
  const chat = mine.chat;
  let data;
  try { data = await invoke("get_note_chat", { id }); } catch (e) { return; }
  if (note !== mine) return;
  const last = data.messages[data.messages.length - 1];
  if (last?.role === "assistant" && last.version && chat.applied !== last.version) {
    chat.applied = last.version;
    const typedSince = chat.sent != null && mine.editorText() !== chat.sent || mine.ed.dictation.pending();
    if (typedSince) {
      chat.notices.set(last.version, "You changed the note while this was being written, so your text was kept. The rewrite is in Versions if you want it.");
      mine.dirty = true;
      await mine.save().catch(() => {});
    } else {
      try { mine.apply(await invoke("get_note", { id })); } catch (e) { /* The note stays as it is. */ }
    }
  }
  Object.assign(chat, data, { loaded: true });
  noteChatStep = null;
  chat.draw();
}

/// Takes back a rewrite: the version before it becomes the note again.
async function undoRewrite(version, status) {
  const mine = note;
  if (!mine?.id) return;
  try {
    await mine.save();
    const all = await invoke("note_versions", { id: mine.id });
    const at = all.findIndex((v) => v.n === version);
    if (at < 1) return showNoteVersions(status, version);
    mine.apply(await invoke("restore_note_version", { id: mine.id, version: all[at - 1].n }));
    mine.chat?.notices.set(version, `Undone: the note is as it was before (version ${all[at - 1].n}).`);
    mine.chat?.draw();
  } catch (e) {
    if (status?.isConnected) status.textContent = String(e);
  }
}

// ---------- Versions

const versionWho = (v) => (v.by === "ai" ? "Rewritten by the assistant" : v.by === "restore" ? v.label || "An earlier version brought back" : "Your writing");
const wordCount = (text) => (text.trim() ? text.trim().split(/\s+/).length : 0);
const searchWords = (query) => query.toLowerCase().split(/\s+/).filter(Boolean);

/// Text with each search word marked.
function marked(text, words) {
  if (!words.length || !text) return [text];
  const escaped = words.map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  const parts = text.split(new RegExp(`(${escaped.join("|")})`, "gi"));
  return parts.map((p, i) => (i % 2 ? el("mark", {}, p) : p));
}

/// The words around the first search word found, to show why a version matched.
function snippet(text, words) {
  const flat = text.replace(/\s+/g, " ").trim();
  if (!words.length) return flat.slice(0, 110);
  const lower = flat.toLowerCase();
  const at = Math.min(...words.map((w) => lower.indexOf(w)).filter((i) => i >= 0), Infinity);
  if (at === Infinity) return flat.slice(0, 110);
  const from = Math.max(0, at - 40);
  return (from ? "…" : "") + flat.slice(from, from + 110);
}

/// What changed between two texts, word by word: [[kind, text]] with kind "same", "added" or "removed".
function wordDiff(before, after) {
  // Words, spaces and punctuation each count, so "venue" becoming "venue," adds only the comma.
  const pieces = (text) => text.split(/(\s+|[.,;:!?()"“”])/).filter(Boolean);
  const a = pieces(before), b = pieces(after);
  if (a.length * b.length > 6e6) return null; // Too long to compare quickly.
  const width = b.length + 1;
  const table = new Uint32Array((a.length + 1) * width);
  for (let i = a.length - 1; i >= 0; i--) {
    for (let j = b.length - 1; j >= 0; j--) {
      table[i * width + j] = a[i] === b[j] ? table[(i + 1) * width + j + 1] + 1 : Math.max(table[(i + 1) * width + j], table[i * width + j + 1]);
    }
  }
  const out = [];
  const push = (kind, text) => { const last = out[out.length - 1]; if (last && last[0] === kind) last[1] += text; else out.push([kind, text]); };
  let i = 0, j = 0;
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) { push("same", a[i]); i++; j++; }
    else if (table[(i + 1) * width + j] >= table[i * width + j + 1]) push("removed", a[i++]);
    else push("added", b[j++]);
  }
  while (i < a.length) push("removed", a[i++]);
  while (j < b.length) push("added", b[j++]);
  return out;
}

/// The versions of the note that's open: a list to step through and search on the left, the version
/// picked on the right, and a button to make it the note again. `focus` picks a version to start on.
async function showNoteVersions(status, focus) {
  const mine = note;
  if (!mine) return;
  let all;
  try {
    const id = await mine.save();
    all = (await invoke("note_versions", { id })).reverse(); // Newest first.
  } catch (e) {
    if (status?.isConnected) status.textContent = String(e);
    return;
  }
  if (note !== mine) return;
  const current = all[0]?.n;
  let picked = all.find((v) => v.n === focus)?.n ?? current;
  let shown = all;
  let changes = localStorage.getItem("version-changes") === "1";

  const search = el("input", { type: "search", placeholder: "Search versions", "aria-label": "Search versions", autocomplete: "off" });
  const list = el("div", { class: "version-list", role: "listbox", "aria-label": "Versions", tabindex: "0" });
  const count = el("div", { class: "hint version-count" });
  const pane = el("div", { class: "version-view" });
  const older = el("button", { class: "pill", type: "button", title: "The version before this one (↓)" }, "‹ Older");
  const newer = el("button", { class: "pill", type: "button", title: "The version after this one (↑)" }, "Newer ›");
  const where = el("span", { class: "hint" });
  const use = el("button", { class: "primary pill", type: "button" });
  const showChanges = el("input", { type: "checkbox", checked: changes });
  const dialog = el("dialog", { class: "versions", "aria-label": "Versions of this note" },
    el("div", { class: "versions-head" }, el("h2", {}, "Versions"),
      el("span", { class: "hint" }, all.length > 1 ? "Each rewrite and each return to the note is kept. Pick one to read it." : "This is the only version so far. A new one is kept each time the assistant rewrites the note, and when you come back to edit it."),
      el("span", { class: "grow" }),
      el("button", { class: "icon-only", type: "button", title: "Close", "aria-label": "Close", onclick: () => dialog.close() }, svg("close", 18))),
    el("div", { class: "versions-body" },
      el("div", { class: "versions-side" }, el("label", { class: "search" }, svg("search", 16), search), count, list),
      el("div", { class: "versions-main" }, pane,
        el("div", { class: "versions-foot" }, older, newer, where,
          el("label", { class: "version-changes", title: "Mark what this version added and took out, compared with the one before it" }, showChanges, "Show changes"),
          el("span", { class: "grow" }), use))));

  const draw = () => {
    const words = searchWords(search.value);
    shown = all.filter((v) => { const hay = `${v.title}\n${v.body}\n${v.label}\n${versionWho(v)}\nversion ${v.n}`.toLowerCase(); return words.every((w) => hay.includes(w)); });
    if (!shown.some((v) => v.n === picked)) picked = shown[0]?.n ?? null;
    count.textContent = words.length ? `${shown.length} of ${all.length} version${all.length === 1 ? "" : "s"}` : "";
    list.replaceChildren(...shown.map((v) => {
      const row = el("button", { class: `version-row ${v.by}`, type: "button", role: "option", "aria-selected": String(v.n === picked), tabindex: "-1" },
        el("span", { class: "version-top" }, el("strong", {}, `Version ${v.n}`), v.n === current ? el("span", { class: "badge now" }, "Current") : null,
          el("span", { class: "grow" }), el("span", { class: "hint", title: dateText(v.at) }, agoText(v.at))),
        el("span", { class: "version-by" }, v.by === "ai" ? svg("sparkle", 13) : null, versionWho(v)),
        v.by === "ai" && v.label ? el("span", { class: "version-asked" }, "“", ...marked(v.label, words), "”") : null,
        el("span", { class: "version-snippet" }, ...marked(snippet(`${v.title} ${v.body}`, words), words)));
      row.addEventListener("click", () => { picked = v.n; draw(); });
      return row;
    }));
    if (!shown.length) list.append(el("p", { class: "hint version-none" }, "No version has those words."));
    list.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" });

    const v = all.find((x) => x.n === picked);
    const at = shown.findIndex((x) => x.n === picked);
    older.disabled = at < 0 || at >= shown.length - 1;
    newer.disabled = at <= 0;
    where.textContent = at < 0 ? "" : `${shown.length - at} of ${shown.length}`;
    use.disabled = !v || v.n === current;
    use.textContent = !v ? "Make this the note" : v.n === current ? "This is the note now" : `Make version ${v.n} the note`;
    use.title = v && v.n !== current ? "It becomes a new version, and the note as it is now stays in this list." : "";
    if (!v) return pane.replaceChildren();
    const before = all[all.findIndex((x) => x.n === v.n) + 1];
    let text = marked(v.body, words);
    let remark = null;
    if (changes && before) {
      const diff = wordDiff(before.body, v.body);
      if (diff) text = diff.map(([kind, t]) => (kind === "same" ? marked(t, words) : el(kind === "added" ? "ins" : "del", {}, ...marked(t, words)))).flat();
      else remark = "This version is too long to compare.";
    } else if (changes) remark = "This is the first version kept, so there is nothing to compare it with.";
    pane.replaceChildren(
      el("div", { class: "version-meta" }, `Version ${v.n} · ${versionWho(v)} · ${dateText(v.at)} · ${wordCount(v.body)} word${wordCount(v.body) === 1 ? "" : "s"}`,
        changes && before ? ` · compared with version ${before.n}` : ""),
      remark ? el("div", { class: "hint" }, remark) : "",
      el("div", { class: "version-title" }, ...marked(v.title || "Untitled note", words)),
      el("div", { class: "version-text" }, ...text));
    pane.scrollTop = 0;
  };
  const step = (by) => {
    const at = shown.findIndex((x) => x.n === picked) + by;
    if (at >= 0 && at < shown.length) { picked = shown[at].n; draw(); }
  };
  older.addEventListener("click", () => step(1));
  newer.addEventListener("click", () => step(-1));
  search.addEventListener("input", draw);
  showChanges.addEventListener("change", () => { changes = showChanges.checked; localStorage.setItem("version-changes", changes ? "1" : ""); draw(); });
  dialog.addEventListener("keydown", (e) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    step(e.key === "ArrowDown" ? 1 : -1);
  });
  use.addEventListener("click", async () => {
    use.disabled = true;
    try {
      const fresh = await invoke("restore_note_version", { id: mine.id, version: picked });
      if (note === mine) mine.apply(fresh);
      dialog.close();
    } catch (e) {
      where.textContent = String(e);
      use.disabled = false;
    }
  });
  dialog.addEventListener("close", () => dialog.remove());
  document.body.append(dialog);
  draw();
  dialog.showModal();
  list.focus();
}
