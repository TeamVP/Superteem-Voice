"use strict";

const { invoke, convertFileSrc } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const main = document.getElementById("main");
const itemsNav = document.getElementById("items");
const search = document.getElementById("search");

const SPEAKER_COLOURS = ["#dbeafe", "#dcfce7", "#fde7d0", "#e0d7f4", "#fce1e4", "#e0f2f1", "#f1f0d4"];
const SPEAKER_COLOURS_DARK = ["#1e3a5f", "#1d4030", "#4a3420", "#2c2145", "#4a2530", "#1f3d3a", "#3d3b20"];
/// Stronger versions for the track timeline, where the blocks are small.
const TRACK_COLOURS = ["#5b82d6", "#3fa36b", "#d9823b", "#a388dd", "#d65a73", "#3a9e96", "#a59a3a"];

/// Whisper writes the app's name many ways. A greeting must come first, or the name must open the phrase,
/// so "we have a super team" isn't taken as the wake word.
const WAKE = /(?:^\W*|\b(?:hey|hi|hay|okay|ok)[\s,.!]+)super[\s-]?t(?:ee|ea|he|i)m?e?s?\b[\s,.!]*/i;

let view = { kind: "home" };
let pollTimer = null;
let stopping = false;
let player = null;
let platform = "windows";
let progress = null;
// Meetings being processed one at a time, while another can be recorded: { current, waiting }.
let queue = { current: null, waiting: [] };
const queued = (id) => queue.current === id || queue.waiting.includes(id);
let prefs = { wake: false, quiet: false, onboarded: true, inputs: [], pins: false, mics: {}, scripts: [] };
let homeMode = "note";

const icon = {
  pen: '<path d="M4 20h4L19 9l-4-4L4 16z"></path>',
  wave: '<path d="M4 9v6M8 6v12M12 10v4M16 7v10M20 10v4"></path>',
  mic: '<rect x="9" y="3" width="6" height="12" rx="3"></rect><path d="M5 11a7 7 0 0014 0M12 18v3"></path>',
  bookmark: '<path d="M6 3h12v18l-6-4-6 4z"></path>',
  action: '<rect x="4" y="4" width="16" height="16" rx="3"></rect><path d="M8 12l3 3 5-6"></path>',
  play: '<path d="M7 4l13 8-13 8z" fill="currentColor" stroke="none"></path>',
  "skip-back": '<path d="M18 5l-10 7 10 7z" fill="currentColor" stroke="none"></path><path d="M6 5v14"></path>',
  "skip-forward": '<path d="M6 5l10 7-10 7z" fill="currentColor" stroke="none"></path><path d="M18 5v14"></path>',
  pause: '<rect x="6" y="5" width="4" height="14" rx="1" fill="currentColor" stroke="none"></rect><rect x="14" y="5" width="4" height="14" rx="1" fill="currentColor" stroke="none"></rect>',
  check: '<path d="M5 12l5 5 9-10"></path>',
  close: '<path d="M6 6l12 12M18 6L6 18"></path>',
  archive: '<path d="M3 5h18v4H3zM5 9v10h14V9M10 13h4"></path>',
  unarchive: '<path d="M3 5h18v4H3zM5 9v10h14V9M12 17v-6M9.5 13.5L12 11l2.5 2.5"></path>',
  trash: '<path d="M4 7h16M10 3h4M6 7l1 13h10l1-13M10 11v5M14 11v5"></path>',
  home: '<path d="M3 11l9-7 9 7M5 10v10h5v-6h4v6h5V10"></path>',
};

function svg(name, size = 18) {
  const span = document.createElement("span");
  span.style.display = "inline-flex";
  span.setAttribute("aria-hidden", "true");
  span.innerHTML = `<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${icon[name]}</svg>`;
  return span;
}

function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (key === "class") node.className = value;
    else if (key.startsWith("on")) node.addEventListener(key.slice(2), value);
    else if (value === true) node.setAttribute(key, "");
    else if (value !== false && value != null) node.setAttribute(key, value);
  }
  for (const child of children.flat()) {
    if (child != null) node.append(child.nodeType ? child : document.createTextNode(String(child)));
  }
  return node;
}

function clock(seconds) {
  const s = Math.max(0, Math.floor(seconds));
  const h = Math.floor(s / 3600);
  const mm = String(Math.floor(s / 60) % 60).padStart(2, "0");
  const ss = String(s % 60).padStart(2, "0");
  return h ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

function longClock(seconds) {
  const s = Math.max(0, Math.floor(seconds));
  return [Math.floor(s / 3600), Math.floor(s / 60) % 60, s % 60].map((n) => String(n).padStart(2, "0")).join(":");
}

function dateText(unix) {
  if (!unix) return "";
  return new Date(unix * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

function timeText(unix) {
  return unix ? new Date(unix * 1000).toLocaleTimeString(undefined, { timeStyle: "short" }) : "";
}

function showError(target, error) {
  target.querySelector(".error")?.remove();
  target.append(el("div", { class: "error", role: "alert" }, String(error)));
}

/// On a phone the list and the page take turns filling the screen.
function showPage() {
  document.body.classList.remove("show-list");
}

/// Leaves whatever page was open: stops playback and polling, and saves a note being edited.
function leave() {
  keyphrasePage?.stop();
  keyphrasePage = null;
  stopProbe();
  cancelOnboardReading();
  cancelDictation();
  stopPlayback();
  clearInterval(pollTimer);
  flushNote();
}

// ---------- Theme: follows Windows by default; a pick is remembered on this device ----------

const systemDark = matchMedia("(prefers-color-scheme: dark)");

function savedTheme() {
  try {
    return localStorage.getItem("theme") || "system";
  } catch {
    return "system";
  }
}

/// Read from the page rather than storage, which can be unavailable.
function isDark() {
  const theme = document.documentElement.dataset.theme;
  return theme === "dark" || (!theme && systemDark.matches);
}

function applyTheme(theme) {
  if (theme === "system") delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = theme;
}

function pickTheme(theme) {
  try {
    localStorage.setItem("theme", theme);
  } catch {}
  invoke("save_prefs", { theme }).catch(() => {});
  applyTheme(theme);
  rerender();
}

systemDark.addEventListener("change", () => rerender());
applyTheme(savedTheme());

/// Speaker colours are set inline, so the open session is redrawn when the theme changes.
function rerender() {
  if (view.kind === "session" && !document.activeElement?.isContentEditable) openSession(view.id);
}

function colourFor(index) {
  const list = isDark() ? SPEAKER_COLOURS_DARK : SPEAKER_COLOURS;
  return list[index % list.length];
}

// ---------- Sidebar: notes and meetings in one list ----------

let searchTimer = null;
let recording = null; // { started: ms, devices: [...] } while a meeting records

function dayLabel(unix) {
  const day = new Date(unix * 1000);
  const today = new Date();
  const start = (d) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const diff = Math.round((start(today) - start(day)) / 86400000);
  if (diff <= 0) return "Today";
  if (diff === 1) return "Yesterday";
  if (diff < 7) return day.toLocaleDateString(undefined, { weekday: "long" });
  return day.toLocaleDateString(undefined, { month: "long", year: "numeric" });
}

/// The person's own workflow for notes and meetings. No status is the default.
const STATUSES = [["", "No status"], ["in_progress", "In progress"], ["ready", "Ready"], ["exported", "Exported"], ["done", "Done"]];
const statusName = (status) => STATUSES.find(([value]) => value === (status ?? ""))?.[1] ?? "No status";

/// A dropdown showing the current status, coloured to match the list.
function statusPicker(status, onChange) {
  const select = el("select", { class: "status-picker", "aria-label": "Status", title: "Status" },
    ...STATUSES.map(([value, label]) => el("option", { value, selected: value === (status ?? "") }, label)));
  select.dataset.status = status ?? "";
  select.addEventListener("change", async () => {
    const previous = select.dataset.status;
    select.dataset.status = select.value;
    if (!(await onChange(select.value))) {
      select.value = previous;
      select.dataset.status = previous;
    }
  });
  return select;
}

/// The status in the list: nothing when unset.
function statusBadge(status) {
  return status ? el("span", { class: "status-badge", "data-status": status }, statusName(status)) : null;
}

/// Titles for meetings nobody named: which meeting of the day it was, then the day, as "1- Monday 9 Nov 2026".
let meetingNames = {};

function isUntitled(title) {
  return !title?.trim() || /^untitled (session|meeting)$/i.test(title.trim());
}

function meetingTitle(id, title) {
  return isUntitled(title) ? meetingNames[id] ?? "Meeting" : title;
}

/// Numbers every meeting within its local day, counting all of them, not only those a search shows.
function nameMeetings(all) {
  const byDay = {};
  for (const s of [...all].sort((a, b) => a.created - b.created)) {
    const day = new Date(s.created * 1000);
    const key = day.toDateString();
    byDay[key] = (byDay[key] ?? 0) + 1;
    const label = day.toLocaleDateString("en-GB", { weekday: "long", day: "numeric", month: "short", year: "numeric" }).replace(",", "");
    meetingNames[s.id] = `${byDay[key]}- ${label}`;
  }
}

async function refreshItems() {
  const query = search.value;
  const [notes, sessions, all] = await Promise.all([
    invoke("list_notes", { query }).catch(() => []),
    invoke("list_sessions", { query }).catch(() => []),
    query.trim() ? invoke("list_sessions", { query: "" }).catch(() => []) : null,
  ]);
  nameMeetings(all ?? sessions);
  const items = [
    // Archived notes are left out of the list but still found by search.
    ...notes.filter((n) => query.trim() || !n.archived).map((n) => ({ kind: "note", id: n.id, title: n.title, when: n.updated, status: n.status,
      sub: [`Note · ${timeText(n.updated)}`, n.words ? `${n.words} words` : null, n.archived ? "Archived" : null].filter(Boolean).join(" · ") })),
    ...sessions.map((s) => ({ kind: "session", id: s.id, title: meetingTitle(s.id, s.title), when: s.created, status: s.status,
      sub: [`Meeting · ${timeText(s.created)}`, s.duration_s ? clock(s.duration_s) : null,
        queue.current === s.id ? "processing…" : queue.waiting.includes(s.id) ? "waiting to process" : null,
        s.needs_processing ? "not processed" : null].filter(Boolean).join(" · ") })),
  ].sort((a, b) => b.when - a.when);

  itemsNav.replaceChildren();
  if (recording) {
    itemsNav.append(el("button", { class: "item" + (view.kind === "recorder" ? " active" : ""), onclick: () => showMeeting() },
      el("span", { class: "icon meeting" }, svg("wave", 16)),
      el("span", { class: "body" }, el("span", { class: "title rec" }, el("span", { class: "dot" }), "Recording now"),
        el("span", { class: "sub" }, "Tap to return"))));
  }
  const archived = notes.filter((n) => n.archived).length;
  const archiveLink = !query.trim() && archived
    ? el("button", { class: "item archive-link" + (view.kind === "archive" ? " active" : ""), onclick: () => showArchive() },
      el("span", { class: "icon" }, svg("archive", 16)),
      el("span", { class: "body" }, el("span", { class: "title" }, "Archive"), el("span", { class: "sub" }, `${archived} note${archived > 1 ? "s" : ""}`)))
    : null;
  if (!items.length) {
    itemsNav.append(el("div", { class: "empty" }, query ? "Nothing matches." : "No notes or meetings yet. Start one above, or say “Hey Superteem”."));
    if (archiveLink) itemsNav.append(archiveLink);
    return;
  }
  let group = null;
  for (const it of items) {
    const label = dayLabel(it.when);
    if (label !== group) {
      group = label;
      itemsNav.append(el("div", { class: "group-label" }, label));
    }
    const active = (view.kind === "note" && it.kind === "note" && view.id === it.id)
      || (view.kind === "session" && it.kind === "session" && view.id === it.id);
    itemsNav.append(el("button", { class: "item" + (active ? " active" : "") + (it.status === "done" ? " done" : ""),
      onclick: () => (it.kind === "note" ? openNote(it.id) : openSession(it.id)) },
      el("span", { class: "icon" + (it.kind === "session" ? " meeting" : "") }, svg(it.kind === "note" ? "pen" : "wave", 16)),
      el("span", { class: "body" }, el("span", { class: "title" }, it.title), el("span", { class: "sub" }, statusBadge(it.status), it.sub))));
  }
  if (archiveLink) itemsNav.append(archiveLink);
}

/// Archived notes, each of which can be opened or put back in the list.
async function showArchive() {
  leave();
  showPage();
  view = { kind: "archive" };
  refreshItems();
  const notes = (await invoke("list_notes", { query: "" }).catch(() => [])).filter((n) => n.archived);
  const page = el("div", { class: "page" }, el("h1", {}, "Archive"),
    el("p", { class: "legend" }, "Archived notes are kept out of the list. Search still finds them."));
  page.append(el("div", { class: "group" }, notes.length ? notes.map((n) => el("div", { class: "row" },
    el("button", { class: "label link-label", onclick: () => openNote(n.id) }, n.title, el("small", {}, `${timeText(n.updated)} · ${n.preview || "Empty"}`)),
    el("button", { class: "pill", onclick: async () => { await invoke("archive_note", { id: n.id, archived: false }); showArchive(); } }, "Unarchive")))
    : el("div", { class: "row" }, el("span", { class: "label" }, "Nothing archived."))));
  main.replaceChildren(page);
}

search.addEventListener("input", () => {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(refreshItems, 200);
});
document.getElementById("new-note").addEventListener("click", () => openNote(null));
// The sidebar button for the mode in use is purple: New note in a note, Meeting in a meeting.
new MutationObserver(() => {
  document.getElementById("new-note").classList.toggle("primary", view.kind === "note");
  document.getElementById("new-meeting").classList.toggle("primary", view.kind === "recorder");
  document.getElementById("open-home").classList.toggle("on", view.kind === "home");
}).observe(main, { childList: true });
document.getElementById("new-meeting").addEventListener("click", () => showMeeting());
document.getElementById("open-settings").addEventListener("click", () => showSettings());
// The wordmark is the Home button too.
const goHome = () => showHome();
document.querySelector(".brand").addEventListener("click", goHome);
document.getElementById("open-home").addEventListener("click", goHome);
// Each example on the Listening page runs as if it had been said.
document.getElementById("agent-say").addEventListener("click", (e) => {
  const link = e.target.closest("[data-say]");
  if (!link) return;
  e.preventDefault();
  run(link.dataset.say);
});
document.getElementById("listen-chip").addEventListener("click", () => openAgent());

// The sidebar is resized by dragging its edge, or with the arrow keys there; the width is remembered on this device.
const SIDEBAR_WIDTH = { min: 236, max: 440, default: 300 };
function setSidebarWidth(px, remember = true) {
  const width = Math.round(Math.min(SIDEBAR_WIDTH.max, Math.max(SIDEBAR_WIDTH.min, px)));
  document.body.style.setProperty("--sidebar-width", `${width}px`);
  if (remember) try { localStorage.setItem("sidebarWidth", String(width)); } catch {}
}
try {
  const saved = Number(localStorage.getItem("sidebarWidth"));
  if (saved) setSidebarWidth(saved, false);
} catch {}
{
  const splitter = document.getElementById("splitter");
  const sidebar = document.querySelector(".sidebar");
  splitter.addEventListener("pointerdown", (e) => {
    e.preventDefault();
    splitter.setPointerCapture(e.pointerId);
    const left = sidebar.getBoundingClientRect().left;
    splitter.classList.add("dragging");
    document.body.classList.add("resizing");
    const move = (m) => setSidebarWidth(m.clientX - left, false);
    const up = (u) => {
      splitter.removeEventListener("pointermove", move);
      splitter.classList.remove("dragging");
      document.body.classList.remove("resizing");
      setSidebarWidth(u.clientX - left);
    };
    splitter.addEventListener("pointermove", move);
    splitter.addEventListener("pointerup", up, { once: true });
    splitter.addEventListener("pointercancel", up, { once: true });
  });
  splitter.addEventListener("dblclick", () => setSidebarWidth(SIDEBAR_WIDTH.default));
  splitter.addEventListener("keydown", (e) => {
    const step = e.key === "ArrowLeft" ? -16 : e.key === "ArrowRight" ? 16 : 0;
    if (!step) return;
    e.preventDefault();
    setSidebarWidth(sidebar.getBoundingClientRect().width + step);
  });
}
document.getElementById("back").addEventListener("click", () => {
  leave();
  view = { kind: "home" };
  document.body.classList.add("show-list");
  refreshItems();
});

// ---------- Home: one button, two modes ----------

function showHome() {
  leave();
  view = { kind: "home" };
  showPage();
  refreshItems();
  const modes = el("div", { class: "segmented", role: "group", "aria-label": "What to capture" });
  for (const [mode, label] of [["note", "Note"], ["meeting", "Meeting"]]) {
    modes.append(el("button", { class: homeMode === mode ? "on" : "", "aria-pressed": String(homeMode === mode),
      onclick: () => { homeMode = mode; showHome(); } }, label));
  }
  const mic = el("button", { class: "big-mic", "aria-label": homeMode === "note" ? "New note" : "Record a meeting",
    onclick: () => (homeMode === "note" ? openNote(null, { dictate: platform !== "android" }) : showMeeting()) }, svg("mic", 36));
  main.replaceChildren(el("div", { class: "home" },
    el("h1", {}, homeMode === "note" ? "Capture a thought" : "Capture a meeting"),
    modes, mic,
    el("div", { class: "hint" }, homeMode === "note"
      ? (platform === "android" ? "Tap to write · or say “Hey Superteem, take a note”" : "Tap to start a note by voice · type as you go")
      : "Tap to set up · or say “Hey Superteem, start meeting”")));
}

// ---------- Notes: a text editor that listens ----------

let note = null; // { id, title, body, dirty }
let noteTimer = null;

/// `dictate` starts with the cursor in the body and dictation running; left out, only a new note does.
async function openNote(id, { dictate } = {}) {
  leave();
  showPage();
  let data = { id: null, title: "", body: "" };
  if (id) {
    try {
      data = await invoke("get_note", { id });
    } catch (e) {
      main.replaceChildren(el("div", { class: "page" }, el("div", { class: "error" }, String(e))));
      return;
    }
  }
  note = { id: data.id, title: data.title, body: data.body, keepAudio: !!data.keep_audio, archived: !!data.archived, status: data.status ?? "", dirty: false };
  view = { kind: "note", id: data.id };
  refreshItems();
  renderNote(data, dictate);
}

function renderNote(data, dictate) {
  const status = el("span", { class: "status", "aria-live": "polite" }, data.updated ? `Saved · ${timeText(data.updated)}` : "New note");
  const remove = el("button", { class: "icon-only danger", title: "Delete", "aria-label": "Delete note" }, svg("trash", 18));
  remove.addEventListener("click", async () => {
    if (!note.id) return showHome();
    if (!remove.dataset.armed) {
      remove.dataset.armed = "1";
      remove.classList.add("armed");
      remove.replaceChildren(svg("trash", 16), "Click again to delete");
      setTimeout(() => {
        delete remove.dataset.armed;
        remove.classList.remove("armed");
        remove.replaceChildren(svg("trash", 18));
      }, 4000);
      return;
    }
    await invoke("delete_note", { id: note.id }).catch(() => {});
    note = null;
    showHome();
  });
  const archive = el("button", { class: "icon-only" });
  const showArchived = () => {
    const label = note.archived ? "Unarchive" : "Archive";
    archive.title = label;
    archive.setAttribute("aria-label", label);
    archive.replaceChildren(svg(note.archived ? "unarchive" : "archive", 18));
  };
  showArchived();
  archive.addEventListener("click", async () => {
    flushNote();
    if (!note.id) return showHome();
    const archived = !note.archived;
    await invoke("archive_note", { id: note.id, archived }).catch(() => {});
    // Archiving puts the note away; unarchiving brings it back into the list and keeps it open.
    if (archived) showArchive();
    else { note.archived = false; showArchived(); refreshItems(); }
  });
  const picker = statusPicker(note.status, async (value) => {
    try {
      // A note gets its file on first save, so a status set first saves it.
      if (!note.id) {
        note.id = await invoke("save_note", { id: null, title: note.title, body: note.body, keepAudio: note.keepAudio });
        view.id = note.id;
        note.dirty = false;
      }
      await invoke("set_note_status", { id: note.id, status: value });
      note.status = value;
      refreshItems();
      return true;
    } catch (e) {
      status.textContent = `Not saved: ${e}`;
      return false;
    }
  });
  const title = el("input", { class: "note-title", placeholder: "Title", value: data.title, "aria-label": "Title" });
  // An editable div rather than a textarea, so a dictation can hold its place in the text.
  const body = el("div", { class: "note-body", id: "note-body", contenteditable: "plaintext-only", role: "textbox", "aria-multiline": "true", "data-placeholder": "Start typing, or press Dictate (Ctrl + Space) to speak up to 2 minutes at a time. The words go where the cursor is when you press the tick.", "aria-label": "Note" });
  body.textContent = data.body;
  const chip = el("div", { class: "dictation-chip", id: "dictation-chip", hidden: true, "aria-live": "polite" });
  const changed = () => {
    note.title = title.value;
    note.body = noteText(body);
    body.classList.toggle("empty", !body.textContent && !body.querySelector(".dictation-slot"));
    note.dirty = true;
    status.textContent = "Editing…";
    clearTimeout(noteTimer);
    noteTimer = setTimeout(() => saveNote(status), 600);
  };
  title.addEventListener("input", changed);
  body.addEventListener("input", changed);
  title.addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); body.focus(); } });

  const talk = el("button", { class: "talk windows-only", id: "talk", onclick: () => startDictation() }, svg("mic", 18), "Dictate");
  // Pressing Dictate leaves the cursor where it was, so the words can go there.
  talk.addEventListener("mousedown", (e) => e.preventDefault());
  const live = el("div", { class: "dictating", id: "dictating", hidden: true, "aria-live": "polite" });

  main.replaceChildren(
    el("div", { class: "topbar" }, status, el("span", { class: "grow" }), picker, archive, remove),
    el("div", { class: "note" }, title, chip, body),
    el("div", { class: "toolbar" }, el("span", { class: "grow" }), talk, live));
  // The cursor always starts in the body; a new note starts dictating straight away.
  body.classList.toggle("empty", !data.body);
  body.focus();
  getSelection().selectAllChildren(body);
  getSelection().collapseToEnd();
  if (dictate ?? (!data.id && platform !== "android")) startDictation();
}

async function saveNote(status) {
  if (!note || !note.dirty) return;
  if (!note.id && !note.title.trim() && !note.body.trim()) return;
  note.dirty = false;
  try {
    const id = await invoke("save_note", { id: note.id, title: note.title, body: note.body, keepAudio: note.keepAudio });
    note.id = id;
    if (view.kind === "note") view.id = id;
    if (status?.isConnected) status.textContent = `Saved · ${timeText(Date.now() / 1000)}`;
    refreshItems();
  } catch (e) {
    note.dirty = true;
    if (status?.isConnected) status.textContent = `Not saved: ${e}`;
  }
}

function flushNote() {
  clearTimeout(noteTimer);
  if (note?.dirty) saveNote(null);
}

/// The note's text: the dictation widget adds nothing, and the editor's closing <br> is not a line.
function noteText(body) {
  let out = "";
  const walk = (node) => {
    for (const n of node.childNodes) {
      if (n.nodeType === Node.TEXT_NODE) out += n.data;
      else if (n.classList?.contains("dictation-slot")) continue;
      else if (n.nodeName === "BR") { if (n !== body.lastChild) out += "\n"; }
      else walk(n);
    }
  };
  walk(body);
  return out;
}

/// The cursor in the note body, or the end of the text when the cursor is elsewhere.
function noteRange(body) {
  const sel = getSelection();
  if (document.activeElement === body && sel.rangeCount && body.contains(sel.getRangeAt(0).commonAncestorContainer)) return sel.getRangeAt(0).cloneRange();
  const range = document.createRange();
  range.selectNodeContents(body);
  range.collapse(false);
  return range;
}

/// The text either side of a range in the note body.
function textAround(body, range) {
  const head = document.createRange();
  head.selectNodeContents(body);
  head.setEnd(range.startContainer, range.startOffset);
  const tail = document.createRange();
  tail.selectNodeContents(body);
  tail.setStart(range.endContainer, range.endOffset);
  return [head.toString(), tail.toString()];
}

/// Holds a dictation's place: a line of its own at the cursor, with what followed the cursor moved to the next line.
function placeDictationSlot(body) {
  const range = noteRange(body);
  range.collapse(false);
  const [before, after] = textAround(body, range);
  const slot = el("span", { class: "dictation-slot", contenteditable: "false" }, el("span", { class: "spinner" }), "Writing it down…");
  const nodes = [];
  if (before && !before.endsWith("\n")) nodes.push(document.createTextNode("\n"));
  nodes.push(slot);
  const tail = document.createTextNode("\n");
  nodes.push(tail);
  // A line at the very end needs a <br> for the cursor to sit on it.
  if (!after) nodes.push(document.createElement("br"));
  for (const n of nodes.reverse()) range.insertNode(n);
  slot.added = nodes.filter((n) => n !== slot);
  // Typing carries on below the widget; a cursor in the title stays there.
  if (document.activeElement === body) {
    const caret = document.createRange();
    caret.setStartAfter(tail);
    caret.collapse(true);
    getSelection().removeAllRanges();
    getSelection().addRange(caret);
  }
  body.dispatchEvent(new Event("input"));
  return slot;
}

/// Takes the widget out again, along with the line breaks it added, when nothing is written.
function removeDictationSlot(slot) {
  if (!slot?.isConnected) return;
  const body = slot.closest(".note-body");
  for (const n of slot.added) if (n.isConnected && (n.nodeName === "BR" || n.data === "\n")) n.remove();
  slot.remove();
  body?.dispatchEvent(new Event("input"));
}

/// Puts words in place of a range, with the spacing and capital a typist would add.
/// The new words show green and fade to the text colour.
function insertWords(body, range, words) {
  words = words.trim();
  if (!words) return;
  const [before, after] = textAround(body, range);
  const sentenceStart = !before.trim() || /[.!?]["')\]]?\s*$/.test(before) || /\n\s*$/.test(before);
  if (sentenceStart) words = words[0].toUpperCase() + words.slice(1);
  else if (/^[A-Z][a-z]/.test(words) && !/^I\b/.test(words)) words = words[0].toLowerCase() + words.slice(1);
  // Whisper ends most phrases with a full stop, which reads wrongly mid-sentence.
  if (after.trim() && !/^[\s]*[A-Z\n]/.test(after)) words = words.replace(/[.]$/, "");
  const lead = before && !/\s$/.test(before) ? " " : "";
  const trail = after && !/^[\s.,;:!?)]/.test(after) ? " " : "";
  const span = el("span", { class: "dictated" }, lead + words + trail);
  // Once faded it is plain text again.
  span.addEventListener("animationend", () => {
    if (!span.isConnected) return;
    const parent = span.parentNode;
    span.replaceWith(...span.childNodes);
    parent.normalize();
  });
  range.deleteContents();
  range.insertNode(span);
  body.dispatchEvent(new Event("input"));
  return span;
}

let dictation = null; // { levelTimer, limitTimer, busy }

/// One dictation runs hands-free for at most this long, then is written down.
const DICTATION_LIMIT_S = 120;

async function startDictation() {
  const body = document.getElementById("note-body");
  if (!body || dictation) return;
  // Nothing goes into the text yet: the words land where the cursor is when the tick is pressed.
  dictation = {};
  document.getElementById("dictation-chip").hidden = true;
  const live = document.getElementById("dictating");
  const left = el("span", { class: "left" }, `${clock(DICTATION_LIMIT_S)} left`);
  const wave = el("span", { class: "wave" }, Array.from({ length: 5 }, () => el("span")));
  const cancel = el("button", { class: "icon-btn cancel", title: "Cancel (Esc). Nothing is written and the audio is not kept.", "aria-label": "Cancel dictation", onclick: () => cancelDictation() }, svg("close", 18));
  const keep = el("button", { class: "icon-btn keep", title: "Done (Ctrl + Space). Write it at the cursor.", "aria-label": "Finish dictation and keep it", onclick: () => stopDictation() }, svg("check", 18));
  // The buttons don't take the cursor, so the tick writes where it was.
  for (const b of [cancel, keep]) b.addEventListener("mousedown", (e) => e.preventDefault());
  live.replaceChildren(el("span", { class: "rec" }, el("span", { class: "dot" })), el("span", {}, "Dictating"), wave, left, cancel, keep);
  live.hidden = false;
  document.getElementById("talk").hidden = true;
  const ends = Date.now() + DICTATION_LIMIT_S * 1000;
  dictation.limitTimer = setInterval(() => {
    const s = Math.max(0, (ends - Date.now()) / 1000);
    left.textContent = `${clock(Math.ceil(s))} left`;
    if (s <= 0) stopDictation();
  }, 250);
  try {
    await invoke("dictate_start");
  } catch (e) {
    endDictationUi();
    dictation = null;
    showDictationError(e);
    return;
  }
  if (!dictation) return;
  syncWake().catch(() => {});
  const bars = [...wave.children];
  dictation.levelTimer = setInterval(async () => {
    const level = await invoke("listen_level").catch(() => 0);
    bars.forEach((b, i) => { b.style.height = `${4 + Math.round(level * 16 * (0.6 + 0.4 * Math.sin(Date.now() / 90 + i)))}px`; });
  }, 100);
}

function endDictationUi() {
  clearInterval(dictation?.levelTimer);
  clearInterval(dictation?.limitTimer);
  const live = document.getElementById("dictating");
  if (live) live.hidden = true;
  const talk = document.getElementById("talk");
  if (talk) talk.hidden = false;
}

/// Writes what was said at the cursor, or at the end of the note when the cursor is elsewhere,
/// keeping the audio when Settings asks.
/// `byVoice` when ended with "Hey Superteem, stop recording", which is then left out of the words.
async function stopDictation(byVoice = false) {
  if (!dictation || dictation.busy) return;
  dictation.busy = true;
  endDictationUi();
  const chip = document.getElementById("dictation-chip");
  const body = document.getElementById("note-body");
  const slot = body ? placeDictationSlot(body) : null;
  try {
    // Kept audio is filed under the note, so the note must be saved as keeping it first.
    const keep = !!prefs.keep_audio && !!note;
    if (keep && (!note.id || !note.keepAudio)) {
      note.keepAudio = true;
      note.id = await invoke("save_note", { id: note.id, title: note.title, body: note.body, keepAudio: true });
      view.id = note.id;
    }
    const heard = await invoke("dictate_stop", { note: keep ? note.id : null });
    const words = byVoice ? withoutStopCommand(heard) : heard;
    if (body && slot?.isConnected && words?.trim()) {
      const range = document.createRange();
      range.selectNode(slot);
      insertWords(body, range, words);
    } else {
      removeDictationSlot(slot);
    }
    if (!words && chip) { chip.hidden = false; chip.textContent = "No speech heard."; setTimeout(() => (chip.hidden = true), 2500); }
  } catch (e) {
    removeDictationSlot(slot);
    showDictationError(e);
  }
  dictation = null;
  syncWake().catch(() => {});
}

/// The words of a dictation without the "Hey Superteem, stop recording" that ended it.
function withoutStopCommand(words) {
  const wake = new RegExp(WAKE.source, "gi");
  let last = null;
  for (const m of words.matchAll(wake)) last = m;
  if (last && STOP_WORDS.test(words.slice(last.index + last[0].length))) return words.slice(0, last.index).trim();
  return words.replace(/[\s,.!]*\b(stop|end|finish)\b[\w\s']*\b(record\w*|note|dictat\w*)\W*$/i, "").trim();
}

/// Throws away the dictation: nothing is written and the audio is dropped.
async function cancelDictation() {
  if (!dictation || dictation.busy) return;
  dictation.busy = true;
  endDictationUi();
  await invoke("dictate_cancel").catch(() => {});
  dictation = null;
  syncWake().catch(() => {});
  document.getElementById("note-body")?.focus();
}

function showDictationError(e) {
  const chip = document.getElementById("dictation-chip");
  if (!chip) return;
  chip.hidden = false;
  chip.replaceChildren(el("span", { class: "danger" }, String(e)), " ",
    String(e).includes("model") ? el("button", { class: "pill", onclick: () => showSettings() }, "Open Settings") : null);
}

// Ctrl + Space starts dictation in a note, and finishes it; Esc cancels it.
window.addEventListener("keydown", (e) => {
  if (view.kind !== "note" || platform === "android") return;
  if (e.code === "Space" && e.ctrlKey) {
    e.preventDefault();
    if (!e.repeat) dictation ? stopDictation() : startDictation();
  } else if (e.key === "Escape" && dictation) {
    e.preventDefault();
    cancelDictation();
  }
});

// ---------- Meetings: setup, live and processing ----------

listen("processing", (e) => {
  progress = e.payload;
  const bar = document.getElementById("process-bar");
  if (bar && view.kind === "processing" && view.id === progress.id) {
    bar.style.width = `${Math.round(progress.fraction * 100)}%`;
    document.getElementById("process-stage").textContent = progress.stage;
  }
});

listen("queue", (e) => {
  queue = e.payload;
  if (progress && progress.id !== queue.current) progress = null;
  // A meeting that was waiting has now started, or moved up the queue.
  if (view.kind === "processing" && queued(view.id)) showProcessing(view.id);
  refreshItems();
});

listen("processed", (e) => {
  const { id, summary } = e.payload;
  if (queue.current === id) queue.current = null;
  if (view.kind === "processing" && view.id === id) openSession(id, summary.Err);
  else refreshItems();
});

/// A meeting being processed, or waiting its turn. Recording another meeting carries on meanwhile.
function showProcessing(id) {
  leave();
  showPage();
  view = { kind: "processing", id };
  const ahead = queue.waiting.indexOf(id) + (queue.current ? 1 : 0);
  const now = queue.current === id;
  const fraction = now && progress?.id === id ? Math.round(progress.fraction * 100) : 0;
  main.replaceChildren(el("div", { class: "page" }, el("h1", {}, now ? "Processing" : "Waiting to process"),
    el("p", {}, el("span", { class: "spinner" }), el("span", { id: "process-stage" },
      now ? (progress?.id === id ? progress.stage : "Starting") : `${ahead} meeting${ahead === 1 ? "" : "s"} ahead`)),
    el("div", { class: "meter wide-meter" }, el("div", { id: "process-bar", style: `width:${fraction}%` })),
    el("p", { class: "legend" }, "Finding who spoke and writing the transcript. The audio is already saved, and you can record another meeting meanwhile."),
    recording ? null : el("div", { class: "actions" }, el("button", { class: "pill", onclick: () => showMeeting() }, "Start another meeting"))));
  refreshItems();
}

/// Processes a session that was never processed (after a crash) or again (after the model was added).
async function processAgain(id) {
  try {
    await invoke("process_session", { id });
    queue = await invoke("processing_queue");
    showProcessing(id);
  } catch (e) {
    await openSession(id, String(e));
  }
}

/// Inputs ticked last time when still connected, else the default microphone.
async function rememberedInputs() {
  const [{ devices, default: preferred }, fresh] = await Promise.all([invoke("list_devices"), invoke("get_prefs")]);
  prefs = fresh;
  const remembered = prefs.inputs.filter((name) => devices.includes(name));
  return { devices, preferred, ticked: remembered.length ? remembered : preferred ? [preferred] : [] };
}

async function showMeeting() {
  leave();
  showPage();
  view = { kind: "recorder" };
  refreshItems();
  if (enrolment) return showEnrolRecording();
  const status = await invoke("recording_status");
  if (status.recording || stopping) return renderRecording(status);
  const [{ devices, ticked }, ready] = await Promise.all([rememberedInputs(), invoke("model_ready")]);

  const page = el("div", { class: "page" }, el("h1", {}, "New meeting"),
    el("p", { class: "legend" }, "A timecoded recording with a lane for each microphone and each voice. Transcribed on this device when you stop."));

  // Tracks are only microphones known to work; finding, testing and hiding devices happens in Devices.
  // Android records through its own service, so there every microphone is offered as it is.
  const testing = platform !== "android";
  const known = testing ? knownMics(devices) : devices;
  const boxes = known.map((name) => {
    const connected = devices.includes(name);
    const box = el("input", { type: "checkbox", disabled: !connected, "aria-label": name });
    box.checked = connected && ticked.includes(name);
    const meter = el("div", { style: "width:0%" });
    const state = el("span", { class: "mic-state" }, connected ? (testing ? "Connecting…" : "") : "Not connected");
    const row = el("label", { class: "mic-row" + (connected ? "" : " absent") }, box,
      el("span", { class: "mic-name" }, name, el("span", { class: "meter" }, meter)), state);
    return { name, box, meter, state, row, connected };
  });
  const pins = el("input", { type: "checkbox", checked: prefs.pins });
  const pinsRow = el("label", { class: "mic-row" }, pins, el("span", { class: "mic-name" }, "Voice Pins (up to 3)"),
    el("span", { class: "mic-state" }, "Connect when recording starts"));
  if (!prefs.pins_added) pins.checked = false;
  page.append(el("div", { class: "section-head" }, el("h2", {}, "Tracks"),
      testing ? el("button", { class: "pill", onclick: () => showDevices(true) }, "Add or test devices") : null),
    el("div", { class: "legend" }, testing
      ? "Each microphone is recorded as its own track. Only microphones that have been heard working are listed."
      : "Connect a Bluetooth headset before starting. The phone may not record its own mic and a headset at the same time."),
    el("div", { class: "mic-list" },
      boxes.map((b) => b.row),
      known.length || !testing ? null : el("div", { class: "empty" }, "No tested microphones yet. ",
        el("button", { class: "link", onclick: () => showDevices(true) }, "Add one in Devices"), "."),
      prefs.pins_added ? pinsRow : null));

  if (!ready) {
    page.append(el("div", { class: "notice" }, "The transcript needs the speech model. You can record now and transcribe later. ",
      el("button", { class: "link", onclick: () => showSettings() }, "Get it in Settings")));
  }

  const start = el("button", { class: "primary pill", disabled: true }, "Start recording");
  const update = () => {
    start.disabled = !(pins.checked || boxes.some((b) => b.box.checked));
  };
  [pins, ...boxes.map((b) => b.box)].forEach((b) => b.addEventListener("change", update));
  update();
  start.addEventListener("click", async () => {
    try {
      await startMeeting(boxes.filter((b) => b.box.checked).map((b) => b.name), pins.checked);
    } catch (e) {
      showError(page, e);
    }
  });
  page.append(el("div", { class: "actions" }, start));
  main.replaceChildren(page);
  const live = boxes.filter((b) => b.connected);
  if (!testing || !live.length) return;

  // Watches each connected track: still connected, picking up sound, or failed.
  await invoke("probe_start", { devices: live.map((b) => b.name) });
  probing = true;
  clearInterval(pollTimer);
  pollTimer = setInterval(async () => {
    const probes = await invoke("probe_status").catch(() => []);
    for (const p of probes) {
      const b = live.find((x) => x.name === p.name);
      if (!b) continue;
      b.meter.style.width = `${Math.round(p.level * 100)}%`;
      if (p.state === "failed") {
        b.box.disabled = true;
        if (b.box.checked) { b.box.checked = false; update(); }
        b.row.className = "mic-row failed";
        b.state.textContent = `Not working: ${p.detail}`;
      } else if (p.state === "listening") {
        b.row.className = "mic-row working";
        b.state.replaceChildren(svg("check", 14), p.level > 0.3 ? " Hearing you" : " Connected");
      }
    }
  }, 150);
}

/// Microphones heard working before, from a test, a voice print or a recording, that aren't hidden.
function knownMics(connected) {
  const worked = new Set(prefs.working_mics ?? []);
  if (prefs.enrol_input) worked.add(prefs.enrol_input);
  for (const [name, r] of Object.entries(prefs.mics ?? {})) if (r.ok && r.last_ok >= r.last_failed) worked.add(name);
  const hidden = new Set(prefs.hidden_mics ?? []);
  // Connected ones first, so the usable tracks are at the top.
  return [...worked].filter((m) => !hidden.has(m) && m !== "Superteem pins")
    .sort((a, b) => connected.includes(b) - connected.includes(a));
}

/// Finds, tests and hides microphones, and adds Voice Pins. `fromTracks` returns to the meeting setup.
async function showDevices(fromTracks = false) {
  leave();
  showPage();
  view = { kind: "devices" };
  const [{ devices }, fresh] = await Promise.all([invoke("list_devices"), invoke("get_prefs")]);
  prefs = fresh;
  const hidden = new Set(prefs.hidden_mics ?? []);
  const saveHidden = async () => {
    await invoke("save_prefs", { hiddenMics: [...hidden] }).catch(() => {});
    showDevices(fromTracks);
  };
  const shown = devices.filter((d) => !hidden.has(d));
  const rows = shown.map((name) => {
    const meter = el("div", { style: "width:0%" });
    const worked = knownMics(devices).includes(name);
    const state = el("span", { class: "mic-state" }, worked ? "Worked before" : "Opening…");
    const hide = el("button", { class: "link", onclick: () => { hidden.add(name); saveHidden(); } }, "Hide");
    const row = el("div", { class: "mic-row device" + (worked ? " working" : " untested") },
      el("span", { class: "mic-name" }, name, el("span", { class: "meter" }, meter)), state, hide);
    if (worked) state.prepend(svg("check", 14), " ");
    return { name, meter, state, row, worked };
  });
  const hiddenList = devices.filter((d) => hidden.has(d)).concat((prefs.hidden_mics ?? []).filter((d) => !devices.includes(d)));
  const pinsOn = el("input", { type: "checkbox", checked: prefs.pins_added, role: "switch" });
  pinsOn.addEventListener("change", async () => {
    await invoke("save_prefs", { pinsAdded: pinsOn.checked }).catch(() => {});
    prefs.pins_added = pinsOn.checked;
  });

  const page = el("div", { class: "page" },
    fromTracks ? el("button", { class: "link back-link", onclick: () => showMeeting() }, "‹ Back to the meeting") : null,
    el("div", { class: "section-head" }, el("h1", {}, "Devices"),
      el("button", { class: "pill", onclick: () => showDevices(fromTracks) }, "Refresh devices")),
    el("p", { class: "legend" }, "Say something to test each microphone. Once it hears you it shows ✓ and can be used as a meeting track. Hide any you never use or that don't work."),
    el("h2", {}, "Microphones"),
    el("div", { class: "mic-list" }, rows.map((r) => r.row),
      shown.length ? null : el("div", { class: "empty" }, "No microphones to show.")),
    el("div", { class: "legend" }, "To add a microphone, plug it in or pair it in Windows Bluetooth settings, then press Refresh devices."),
    hiddenList.length ? el("details", { class: "hidden-mics" }, el("summary", {}, `Hidden (${hiddenList.length})`),
      el("div", { class: "mic-list" }, hiddenList.map((name) => el("div", { class: "mic-row device untested" },
        el("span", { class: "mic-name" }, name), el("span", { class: "mic-state" }, devices.includes(name) ? "" : "Not connected"),
        el("button", { class: "link", onclick: () => { hidden.delete(name); saveHidden(); } }, "Show"))))) : null,
    el("h2", {}, "Voice Pins"),
    el("div", { class: "group" }, el("label", { class: "row" },
      el("span", { class: "label" }, "Use Voice Pins", el("small", {}, "Up to 3 pins connect over Bluetooth when a recording starts, each as its own track.")),
      pinsOn)),
    fromTracks ? el("div", { class: "actions" }, el("button", { class: "primary pill", onclick: () => showMeeting() }, "Done")) : null);
  main.replaceChildren(page);
  if (!shown.length) return;

  await invoke("probe_start", { devices: shown });
  probing = true;
  clearInterval(pollTimer);
  pollTimer = setInterval(async () => {
    const probes = await invoke("probe_status").catch(() => []);
    for (const p of probes) {
      const r = rows.find((x) => x.name === p.name);
      if (!r) continue;
      r.meter.style.width = `${Math.round(p.level * 100)}%`;
      if (p.state === "failed") {
        r.row.className = "mic-row device failed";
        r.state.textContent = `Not working: ${p.detail}`;
      } else if (p.heard || r.worked) {
        r.row.className = "mic-row device working";
        r.state.replaceChildren(svg("check", 14), p.heard ? " Working" : " Worked before");
      } else if (p.state === "listening") {
        r.state.textContent = "Say something to test it";
      }
    }
  }, 150);
}

let probing = false;

function stopProbe() {
  if (!probing) return;
  probing = false;
  invoke("probe_stop").catch(() => {});
}

async function startMeeting(devices, pins) {
  clearInterval(pollTimer);
  probing = false;
  await invoke("start_recording", { devices, pins });
  // Consent is asked now that the recording is running: { at_s, how, words } once given.
  recording = { started: Date.now(), devices, markers: [], levels: {}, consent: null };
  view = { kind: "recorder" };
  syncWake().catch(() => {});
  showPage();
  refreshItems();
  renderRecording(await invoke("recording_status"));
}

/// Readable names for streams: inputs are numbered in the order chosen.
function streamName(stream, devices = recording?.devices ?? []) {
  const input = /^input(\d+)$/.exec(stream);
  if (input) return devices[Number(input[1]) - 1] ?? `Input ${input[1]}`;
  const pin = /^pin(\d+)$/.exec(stream);
  if (pin) return `Superteem pin ${pin[1]}`;
  return stream;
}

function nowInRecording() {
  return recording ? (Date.now() - recording.started) / 1000 : 0;
}

/// `kind` is "action" for an action item; a bookmark has none. The item itself is taken from the transcript later.
function addMarker(text, kind = "") {
  if (!recording) return false;
  recording.markers.push({ at_s: Math.round(nowInRecording() * 10) / 10, text, ...(kind ? { kind } : {}) });
  drawMarkers();
  return true;
}

/// Markers saved before action items had a kind were noted as text.
const isAction = (m) => m.kind === "action" || (!m.kind && m.text === "Action item");
const markerLabel = (m) => (isAction(m) ? "Action item" : "Bookmark");

function drawMarkers() {
  const list = document.getElementById("live-markers");
  if (!list || !recording) return;
  list.replaceChildren(...recording.markers.slice(-4).map((m) =>
    el("div", { class: `marker${isAction(m) ? " action" : ""}` }, el("span", { class: "mono" }, clock(m.at_s)),
      el("strong", {}, markerLabel(m)), m.text && m.text !== "Action item" ? ` · ${m.text}` : "")));
}

function renderRecording(status) {
  clearInterval(pollTimer);
  if (stopping) {
    main.replaceChildren(el("div", { class: "page" }, el("h1", {}, "Saving"), el("p", {}, el("span", { class: "spinner" }), "Stopping the inputs")));
    return;
  }
  // A recording found running after the window reloaded.
  recording ??= { started: Date.now() - status.elapsed_s * 1000, devices: prefs.inputs, markers: [], levels: {}, consent: null };
  recording.started = Date.now() - status.elapsed_s * 1000;
  // The clock shows once consent is given; until then the question is what matters.
  const time = el("div", { class: "timecode mono", id: "live-time", "aria-live": "off", hidden: !recording.consent }, longClock(status.elapsed_s));
  const lanes = el("div", { class: "lanes", "aria-label": "Tracks" });
  // Press the bookmark or Enter to mark now, or the tick for an action item, with whatever note is typed.
  const pinned = el("input", { placeholder: "Mark now, with a note if you like: Enter bookmarks it", "aria-label": "Note pinned to the current time" });
  const at = el("span", { class: "mono" }, `@${clock(status.elapsed_s)}`);
  const mark = (kind) => {
    addMarker(pinned.value.trim(), kind);
    pinned.value = "";
  };
  pinned.addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); mark(e.ctrlKey ? "action" : ""); } });
  const bookmark = el("button", { class: "icon-btn bookmark", type: "button", "aria-label": "Bookmark this moment", title: "Bookmark this moment (Enter)", onclick: () => mark("") }, svg("bookmark", 18));
  const actionItem = el("button", { class: "icon-btn action-item", type: "button", "aria-label": "Mark an action item", title: "Mark an action item (Ctrl + Enter). It's written up from what was said here.", onclick: () => mark("action") }, svg("action", 18));
  const hint = meetingListening()
    ? "Say “Bookmark that”, “Action item” or “Hey Superteem, stop recording”"
    : "Turn on “Hey Superteem” in Settings to mark and stop by voice";
  const stop = el("button", { class: "stop", "aria-label": "Stop recording", title: "Stop recording" }, el("span"));
  stop.addEventListener("click", () => stopMeeting());
  const recLabel = el("span", {}, recording.consent ? "REC · consent noted" : "REC · asking for consent");
  main.replaceChildren(el("div", { class: "live" },
    el("div", { class: "live-head" }, el("span", { class: "rec" }, el("span", { class: "dot" }), recLabel),
      el("span", { class: "hint" }, hint)),
    recording.consent ? null : consentChallenge(recLabel),
    time, lanes, el("div", { class: "markers", id: "live-markers" }),
    el("div", { class: "pinned" }, bookmark, actionItem, at, pinned),
    el("div", { class: "live-controls" }, stop)));
  drawMarkers();

  const draw = (s) => {
    time.textContent = longClock(s.elapsed_s);
    at.textContent = `@${clock(nowInRecording())}`;
    lanes.replaceChildren(...s.streams.map((st, i) => {
      const history = (recording.levels[st.name] ??= []);
      history.push(st.level);
      if (history.length > 80) history.shift();
      const colour = TRACK_COLOURS[i % TRACK_COLOURS.length];
      const failing = st.state === "failed" || st.state === "disconnected";
      return el("div", { class: "lane" },
        el("span", { class: "name", style: `color:${colour}` }, streamName(st.name),
          el("small", { class: failing ? "danger" : "" }, failing ? st.detail || st.state : st.state)),
        el("span", { class: "spark" }, history.map((l) => el("span", { style: `height:${Math.max(2, Math.round(l * 24))}px;background:${l > 0.35 ? colour : "var(--line)"}` }))));
    }));
    if (!s.streams.length) lanes.replaceChildren(el("div", { class: "legend" }, "Starting the inputs…"));
  };
  draw(status);
  pollTimer = setInterval(async () => {
    if (view.kind !== "recorder") return;
    const s = await invoke("recording_status");
    if (s.recording) draw(s);
  }, 250);
}

/// Asks, with the recording already running, whether everyone agreed. Yes is noted with the
/// recording; no stops it and throws it away.
function consentChallenge(recLabel) {
  const card = el("div", { class: "consent-card consent-challenge", id: "consent-challenge", role: "alert" },
    el("strong", {}, "Has everyone being recorded agreed to it?"),
    el("div", { class: "legend" }, "Recording has started. Ask the room, then answer here. The answer is kept with the recording."),
    el("div", { class: "actions" },
      el("button", { class: "pill", onclick: () => refuseConsent() }, "No, stop and discard"),
      el("button", { class: "primary pill", onclick: () => giveConsent("button") }, "Yes, everyone agreed")));
  card.recLabel = recLabel;
  return card;
}

/// Notes consent at this moment in the recording. `words` is what was heard for a spoken yes.
function giveConsent(how, words = "") {
  if (!recording || recording.consent) return false;
  recording.consent = { at_s: Math.round(nowInRecording() * 10) / 10, how, words };
  const card = document.getElementById("consent-challenge");
  if (card) { card.recLabel.textContent = "REC · consent noted"; card.remove(); }
  const time = document.getElementById("live-time");
  if (time) time.hidden = false;
  return true;
}

async function refuseConsent() {
  if (!recording || stopping) return;
  const id = await stopMeeting({ discard: true });
  if (id) await invoke("delete_session", { id }).catch(() => {});
}

async function stopMeeting({ discard = false } = {}) {
  if (!recording || stopping) return;
  clearInterval(pollTimer);
  const markers = recording.markers;
  const consent = recording.consent;
  stopping = true;
  view = { kind: "recorder" };
  showPage();
  renderRecording({});
  let id;
  try {
    // Returns at once: the meeting joins the processing queue, so another can be recorded now.
    id = await invoke("stop_recording", { discard });
  } catch (e) {
    stopping = false;
    recording = null;
    syncWake().catch(() => {});
    showMeeting();
    return;
  }
  stopping = false;
  recording = null;
  syncWake().catch(() => {});
  if (discard) {
    showMeeting();
    return id;
  }
  if (consent) await invoke("save_consent", { id, consent }).catch(() => {});
  if (markers.length) await invoke("save_markers", { id, markers }).catch(() => {});
  queue = await invoke("processing_queue").catch(() => queue);
  showProcessing(id);
  return id;
}

/// How an input did last time on this device, so a microphone that failed is noticed before recording.
function micNote(record) {
  if (!record || (!record.ok && !record.failed)) return null;
  if (record.last_failed > record.last_ok) {
    return el("span", { class: "mic-note bad", title: record.last_problem },
      `didn't record last time${record.last_problem ? ` (${record.last_problem})` : ""}`);
  }
  return el("span", { class: "mic-note" }, `worked ${dateText(record.last_ok)}`);
}

// ---------- Meeting review: transcript and tracks share one playhead ----------

function stopPlayback() {
  if (player) {
    for (const a of player.audios.values()) a.pause();
    clearInterval(player.timer);
    player = null;
  }
}

function makePlayer(session, duration, onTick) {
  const audios = new Map();
  for (const [stream, path] of Object.entries(session.audio)) {
    const audio = new Audio(convertFileSrc(path));
    audio.preload = "auto";
    audios.set(stream, audio);
  }
  const p = { audios, t: 0, playing: false, timer: null, duration };
  const master = () => [...audios.values()][0];
  p.seek = (t) => {
    p.t = Math.max(0, Math.min(duration, t));
    for (const a of audios.values()) a.currentTime = p.t;
    onTick(p.t, p.playing);
  };
  p.play = () => {
    if (!audios.size) return;
    for (const a of audios.values()) { a.currentTime = p.t; a.play().catch(() => {}); }
    p.playing = true;
    clearInterval(p.timer);
    p.timer = setInterval(() => {
      p.t = master().currentTime;
      if (master().ended || p.t >= duration) p.pause();
      onTick(p.t, p.playing);
    }, 100);
    onTick(p.t, true);
  };
  p.pause = () => {
    for (const a of audios.values()) a.pause();
    p.playing = false;
    clearInterval(p.timer);
    onTick(p.t, false);
  };
  p.toggle = () => (p.playing ? p.pause() : p.play());
  return p;
}

/// Words, not just Whisper's "[BLANK_AUDIO]" or "(silence)" markers or punctuation.
function hasWords(text) {
  return /[\p{L}\p{N}]/u.test((text ?? "").replace(/\[[^\]]*\]|\([^)]*\)/g, ""));
}

async function openSession(id, processingError) {
  if (queued(id)) return showProcessing(id);
  leave();
  showPage();
  view = { kind: "session", id };
  refreshItems();
  let session;
  try {
    session = await invoke("get_session", { id });
  } catch (e) {
    main.replaceChildren(el("div", { class: "page" }, el("div", { class: "error" }, String(e))));
    return;
  }
  renderSession(session, processingError);
}

let sessionListeners = null;

/// `suggestion` is lines that sound like one just moved: { lines, speaker }.
function renderSession(session, processingError, suggestion) {
  const { id, meta } = session;
  // Silence and noise have no words; only people talking are shown. Meetings processed before
  // silence was dropped still have these lines.
  if (session.transcribed) session.segments = session.segments.filter((s) => s.edited || hasWords(s.text));
  window.currentPrints = session.voiceprints;
  const keepTime = player?.t ?? 0;
  stopPlayback();
  const save = async (command, args) => {
    try {
      await invoke(command, { id, ...args });
      return true;
    } catch (e) {
      showError(page, e);
      return false;
    }
  };
  const reload = async (next) => {
    const fresh = await invoke("get_session", { id });
    renderSession(fresh, null, next);
    refreshItems();
  };

  const page = el("div", { class: "page" });
  const title = el("h1", { contenteditable: "plaintext-only", spellcheck: "false" }, meetingTitle(id, meta.title));
  title.addEventListener("keydown", (e) => {
    if (e.key === "Enter") { e.preventDefault(); title.blur(); }
  });
  title.addEventListener("blur", async () => {
    const value = title.textContent.trim();
    if (value !== meetingTitle(id, meta.title) && (await save("rename_session", { title: value }))) {
      meta.title = value;
      refreshItems();
    }
  });

  const remove = el("button", { class: "link danger" }, "Delete");
  remove.addEventListener("click", async () => {
    if (!remove.dataset.armed) {
      remove.dataset.armed = "1";
      remove.textContent = platform === "android" ? "Tap again to delete permanently" : "Click again to move to the Recycle Bin";
      setTimeout(() => { delete remove.dataset.armed; remove.textContent = "Delete"; }, 4000);
      return;
    }
    stopPlayback();
    if (await save("delete_session", {})) showHome();
  });
  const edits = session.segments.filter((s) => s.edited).length;
  const reprocess = el("button", { class: "link", title: "Run speaker detection and transcription again" }, "Transcribe again");
  reprocess.addEventListener("click", () => processAgain(id));
  const used = new Set(session.segments.map((s) => s.speaker));
  const streams = Object.keys(session.audio).sort();
  const picker = statusPicker(meta.status, async (status) => {
    if (!(await save("set_session_status", { status }))) return false;
    meta.status = status;
    refreshItems();
    return true;
  });
  page.append(title, el("div", { class: "meta" },
    picker,
    el("span", {}, dateText(meta.created)),
    meta.duration_s ? el("span", {}, clock(meta.duration_s)) : null,
    used.size ? el("span", {}, `${used.size} voice${used.size > 1 ? "s" : ""}`) : null,
    streams.length ? el("span", {}, `${streams.length} track${streams.length > 1 ? "s" : ""}`) : null,
    platform === "windows" ? el("button", { class: "link", onclick: () => invoke("open_folder", { id }) }, "Open folder") : null,
    session.model_ready && !session.needs_processing ? reprocess : null,
    remove));

  const processNow = el("button", { class: "primary" }, "Process now");
  processNow.addEventListener("click", () => processAgain(id));
  if (session.needs_processing) {
    page.append(el("div", { class: "notice" },
      el("p", {}, "This recording was saved but not processed, perhaps because the app closed while recording. Its audio is safe."),
      processNow));
  } else if (processingError) {
    page.append(el("div", { class: "notice" }, `Processing failed: ${processingError}`));
  } else if (!session.segments.length) {
    page.append(el("div", { class: "notice" }, "No speech was found in this recording."));
  } else if (!session.transcribed) {
    page.append(el("div", { class: "notice" }, session.model_ready
      ? "This meeting has no transcript yet. Use Transcribe again above."
      : "This meeting has no transcript. Get the speech model in Settings, then use Transcribe again."));
  }
  if (edits) {
    page.append(el("div", { class: "legend" }, `${edits} line${edits > 1 ? "s" : ""} edited by hand. Transcribing again keeps them.`));
  }

  // Speakers: renaming here changes every line for that person.
  const colours = new Map(session.speakers.map((s, i) => [s.key, colourFor(i)]));
  const strong = new Map(session.speakers.map((s, i) => [s.key, TRACK_COLOURS[i % TRACK_COLOURS.length]]));
  const speakerBar = el("div", { class: "speakers" });
  const speakerForm = el("div", { id: "speaker-form" });
  const usedSpeakers = session.speakers.filter((s) => used.has(s.key));
  for (const sp of usedSpeakers) {
    const input = el("input", { value: sp.name, title: "Rename everywhere in this meeting", "aria-label": `Rename ${sp.name}` });
    input.addEventListener("keydown", (e) => { if (e.key === "Enter") input.blur(); });
    input.addEventListener("change", async () => {
      if (await save("rename_speaker", { key: sp.key, name: input.value })) reload();
    });
    const badge = sp.print
      ? el("span", { class: "badge", title: sp.print.score == null ? "Linked to this voice print"
        : "Recognised from the voice print. Rename or use ⋯ if it is wrong." },
        sp.print.score == null ? "voice print" : `recognised ${Math.round(sp.print.score * 100)}%`)
      : null;
    const menu = speakerMenu(sp, usedSpeakers, (action) => speakerAction(action, sp));
    speakerBar.append(el("span", { class: "chip", style: `background:${colours.get(sp.key)}` }, input, badge, menu));
  }
  if (used.size) {
    // How to read and fix the transcript, behind a "?" so it isn't in the way once known.
    const help = el("div", { class: "legend", hidden: true, id: "transcript-help" }, "Rename a voice to update every line, or use ⋯ to merge voices or link a voice print. "
      + "Dotted lines are less certain; a side bar means two people spoke at once.");
    const helpButton = el("button", { class: "icon-only help-toggle", type: "button", title: "How to read and fix the transcript",
      "aria-label": "How to read and fix the transcript", "aria-expanded": "false", "aria-controls": "transcript-help" }, "?");
    helpButton.addEventListener("click", () => {
      help.hidden = !help.hidden;
      helpButton.setAttribute("aria-expanded", String(!help.hidden));
      helpButton.classList.toggle("on", !help.hidden);
    });
    speakerBar.append(helpButton);
    page.append(speakerBar, speakerForm, help);
  }

  /// Handles the ⋯ menu beside a speaker's name.
  async function speakerAction(action, sp) {
    speakerForm.replaceChildren();
    if (action.startsWith("merge:")) {
      if (await save("merge_speakers", { from: sp.key, into: action.slice(6) })) reload();
    } else if (action === "unlink") {
      if (await save("link_speaker", { key: sp.key, print: null, learn: false })) reload();
    } else if (action.startsWith("link:")) {
      const print = action.slice(5);
      if (!(await save("link_speaker", { key: sp.key, print, learn: false }))) return;
      await reload();
      // Teaching the print is offered rather than automatic, so a wrongly grouped speaker can't spoil it.
      const target = main.querySelector("#speaker-form") ?? speakerForm;
      if (sp.voiced_s >= 2) {
        const learn = el("button", { class: "primary" }, "Yes, learn from this meeting");
        learn.addEventListener("click", async () => {
          try {
            await invoke("link_speaker", { id, key: sp.key, print, learn: true });
            target.replaceChildren(el("div", { class: "inline-form" }, "Done. The voice print will recognise them better next time."));
          } catch (e) { showError(target, e); }
        });
        target.replaceChildren(el("div", { class: "inline-form" },
          el("span", {}, `Linked. Also teach the voice print how they sounded here (${Math.round(sp.voiced_s)} s of speech)?`),
          learn, el("button", { onclick: () => target.replaceChildren() }, "No")));
      }
    } else if (action === "new-print") {
      const name = el("input", { type: "text", value: sp.name.startsWith("Guest") || sp.name.startsWith("input") ? "" : sp.name, placeholder: "Their name" });
      const consent = el("input", { type: "checkbox" });
      const create = el("button", { class: "primary" }, "Save voice print");
      create.addEventListener("click", async () => {
        if (await save("save_speaker_print", { key: sp.key, name: name.value, consent: consent.checked })) reload();
      });
      speakerForm.append(el("div", { class: "inline-form" }, el("strong", {}, "New voice print from this speaker:"), name,
        el("label", { class: "field" }, consent, "They agreed to have a voice print kept"), create,
        el("button", { onclick: () => speakerForm.replaceChildren() }, "Cancel")));
      name.focus();
    }
  }

  // Lines that sound like one the person just moved, offered as one step.
  if (suggestion?.lines.length) {
    const target = session.speakers.find((s) => s.key === suggestion.speaker);
    const n = suggestion.lines.length;
    const accept = el("button", { class: "primary" }, `Move ${n === 1 ? "it" : `all ${n}`} to ${target?.name ?? "them"}`);
    accept.addEventListener("click", async () => {
      if (await save("move_lines", { segments: suggestion.lines, speaker: suggestion.speaker })) reload();
    });
    page.append(el("div", { class: "suggestion" },
      el("span", {}, `${n} other line${n > 1 ? "s" : ""} sound${n > 1 ? "" : "s"} like the one you moved (outlined below).`),
      accept, el("button", { onclick: () => reload() }, "Leave them")));
  }

  const duration = meta.duration_s || Math.max(1, ...session.segments.map((s) => s.end_s));
  const lineEls = [];
  const transcript = el("div", { class: "transcript" });
  const allMarkers = sessionMarkers(meta.markers ?? [], session.keyphrases ?? []);
  const markers = [...allMarkers];
  let previous = null;
  const playFrom = (t) => { player?.seek(t); player?.play(); };
  const markerLine = (m) => el("div", { class: `line marker-line${isAction(m) ? " action" : ""}` },
    el("button", { class: "time mono", title: "Play from here", onclick: () => playFrom(m.at_s) }, clock(m.at_s)),
    el("div", {}, el("strong", {}, markerLabel(m)), markerNote(m, session.segments) ? ` · ${markerNote(m, session.segments)}` : "",
      m.heard ? el("span", { class: "edited", title: "Heard in the recording when it was processed" }, "heard") : null));

  // Action items, written up from what was said where each was marked.
  const actions = allMarkers.filter(isAction);
  if (actions.length) {
    page.append(el("h2", {}, `Action item${actions.length > 1 ? "s" : ""}`), el("ul", { class: "action-items" },
      actions.map((m) => el("li", {},
        el("button", { class: "time mono", title: "Play from here", onclick: () => playFrom(m.at_s) }, clock(m.at_s)),
        el("span", {}, markerNote(m, session.segments) || el("span", { class: "hint" }, "Nothing was said here to write up."))))));
  }
  for (const seg of session.segments) {
    while (markers.length && markers[0].at_s <= seg.start_s) {
      transcript.append(markerLine(markers.shift()));
      previous = null;
    }
    const select = el("select", { style: `background:${colours.get(seg.speaker)}`, title: "Change the speaker for this line" },
      session.speakers.map((sp) => el("option", { value: sp.key, selected: sp.key === seg.speaker }, sp.name)),
      el("option", { value: "__new" }, "New person…"));
    select.addEventListener("change", async () => {
      let key = select.value;
      if (key === "__new") {
        const name = await askName(select);
        if (!name) { select.value = seg.speaker; return; }
        key = await invoke("add_speaker", { id, name });
      }
      try {
        const similar = await invoke("update_segment", { id, segment: seg.id, speaker: key, text: null });
        reload({ lines: similar, speaker: key });
      } catch (e) {
        showError(page, e);
      }
    });
    const text = el("div", { class: "text", contenteditable: "plaintext-only", spellcheck: "true" }, seg.text);
    text.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); text.blur(); }
      if (e.key === "Escape") { text.textContent = seg.text; text.blur(); }
    });
    text.addEventListener("blur", async () => {
      const value = text.textContent.trim();
      if (value !== seg.text && (await save("update_segment", { segment: seg.id, text: value, speaker: null }))) {
        seg.text = value;
        seg.edited = true;
        if (!who.querySelector(".edited")) who.append(el("span", { class: "edited" }, "edited"));
      }
    });
    const who = el("div", { class: "who" }, select,
      seg.overlap ? el("span", { class: "mark", title: "Someone else was speaking at the same time" }, "talking at once") : null,
      seg.edited ? el("span", { class: "edited" }, "edited") : null);
    const low = seg.confidence < session.low_confidence;
    const suggested = suggestion?.lines.includes(seg.id);
    // An overlapping line always shows its speaker, since the one before may be the other voice.
    const same = previous === seg.speaker && !seg.overlap;
    const line = el("div", { class: `line${low ? " low" : ""}${same ? " same-speaker" : ""}${seg.overlap ? " overlap" : ""}${suggested ? " suggest" : ""}` });
    const time = el("button", { class: "time mono", title: "Play from this line" }, clock(seg.start_s));
    time.addEventListener("click", () => {
      if (!player) return;
      player.seek(seg.start_s);
      player.play();
    });
    const action = el("div", { class: "line-action" });
    line.append(time, el("div", {}, who, text), action);
    transcript.append(line);
    lineEls.push({ seg, line, text, action });
    previous = seg.speaker;
  }
  markers.forEach((m) => transcript.append(markerLine(m)));
  page.append(transcript);

  // SPLIT where the cursor sits mid-line; JOIN at the start (onto the line before, taking its speaker)
  // or at the end (the next line joins this one). JOIN is greyed out when the lines can't be joined.
  const joinBlocker = (a, b) => !a || !b ? "There is no line to join"
    : a.stream !== b.stream ? "These lines were heard on different microphones"
    : a.overlap || b.overlap ? "Someone else was talking at the same time" : null;
  const actionButton = (label, title, disabled, run) => el("button", {
    type: "button", title, disabled,
    // Keeps the cursor in the text, so the line isn't saved and redrawn before the click lands.
    onmousedown: (e) => e.preventDefault(),
    onclick: run,
  }, label);
  const applyLineAction = async (entry, command, args) => {
    const value = entry.text.textContent.trim();
    if (value !== entry.seg.text && !(await save("update_segment", { segment: entry.seg.id, text: value, speaker: null }))) return;
    if (await save(command, args)) reload();
  };
  const showLineAction = () => {
    for (const e of lineEls) e.action.replaceChildren();
    const sel = getSelection();
    if (!sel.rangeCount || !sel.isCollapsed) return;
    const i = lineEls.findIndex((e) => e.text.contains(sel.anchorNode));
    if (i < 0) return;
    const entry = lineEls[i];
    const range = document.createRange();
    range.selectNodeContents(entry.text);
    range.setEnd(sel.anchorNode, sel.anchorOffset);
    const before = range.toString();
    const after = entry.text.textContent.slice(before.length);
    if (!before.trim()) {
      const prev = lineEls[i - 1]?.seg;
      const why = joinBlocker(prev, entry.seg);
      entry.action.append(actionButton("JOIN", why ?? "Join this line onto the one before; it takes that speaker", !!why,
        () => applyLineAction(entry, "join_segments", { first: prev.id, second: entry.seg.id })));
    } else if (!after.trim()) {
      const next = lineEls[i + 1]?.seg;
      const why = joinBlocker(entry.seg, next);
      entry.action.append(actionButton("JOIN", why ?? "Join the next line onto this one; it takes this speaker", !!why,
        () => applyLineAction(lineEls[i + 1], "join_segments", { first: entry.seg.id, second: next.id })));
    } else {
      // The split is counted in the trimmed text the backend holds.
      const at = [...before.replace(/^\s+/, "")].length;
      entry.action.append(actionButton("SPLIT", "Split this line here, so the rest can be given to another speaker", false,
        () => applyLineAction(entry, "split_segment", { segment: entry.seg.id, at })));
    }
  };
  // One listener per drawn meeting; redrawing or leaving drops the old one.
  sessionListeners?.abort();
  sessionListeners = new AbortController();
  const signal = sessionListeners.signal;
  document.addEventListener("selectionchange", () => {
    if (!page.isConnected) return sessionListeners.abort();
    showLineAction();
  }, { signal });

  // The tracks: one lane per recorded input, all on one playhead.
  const tracks = el("div", { class: "tracks" });
  if (streams.length) {
    const head = el("span", { class: "playhead" });
    const timeLabel = el("span", { class: "mono hint" });
    const playButton = el("button", { class: "play", "aria-label": "Play" }, svg("play", 18));
    const lanes = el("div", { class: "track-lanes", title: "Click to jump to a moment" });
    for (const stream of streams) {
      const speaker = session.speakers.find((s) => s.key === stream);
      const name = speaker?.name ?? streamName(stream, prefs.inputs);
      const mute = el("button", { class: "mute", "aria-pressed": "false", title: `Mute ${name}` }, "M");
      mute.addEventListener("click", (e) => {
        e.stopPropagation();
        const audio = player?.audios.get(stream);
        if (!audio) return;
        audio.muted = !audio.muted;
        mute.classList.toggle("on", audio.muted);
        mute.setAttribute("aria-pressed", String(audio.muted));
      });
      const bar = el("span", { class: "bar" },
        session.segments.filter((s) => s.stream === stream).map((s) => el("span", {
          style: `left:${(100 * s.start_s) / duration}%;width:${Math.max(0.3, (100 * (s.end_s - s.start_s)) / duration)}%;background:${strong.get(s.speaker) ?? "var(--key-light)"}`,
          title: `${session.speakers.find((sp) => sp.key === s.speaker)?.name ?? ""} · ${clock(s.start_s)}`,
        })),
        allMarkers.map((m) => el("i", { class: isAction(m) ? "action" : "", style: `left:${(100 * m.at_s) / duration}%`, title: `${markerLabel(m)} · ${clock(m.at_s)}` })));
      lanes.append(el("div", { class: "track" }, el("span", { class: "name", style: `color:${strong.get(stream) ?? "inherit"}` }, name), mute, bar));
    }
    lanes.append(head);
    lanes.addEventListener("click", (e) => {
      const bar = lanes.querySelector(".bar").getBoundingClientRect();
      const fraction = (e.clientX - bar.left) / bar.width;
      if (fraction >= 0 && fraction <= 1) player?.seek(fraction * duration);
    });
    const onTick = (t, playing) => {
      const bar = lanes.querySelector(".bar");
      if (bar) {
        const lanesRect = lanes.getBoundingClientRect();
        const rect = bar.getBoundingClientRect();
        head.style.left = `${rect.left - lanesRect.left + (rect.width * t) / duration}px`;
      }
      timeLabel.textContent = `${clock(t)} / ${clock(duration)}`;
      playButton.replaceChildren(svg(playing ? "pause" : "play", 18));
      playButton.setAttribute("aria-label", playing ? "Pause" : "Play");
      for (const { seg, line } of lineEls) line.classList.toggle("playing", playing && t >= seg.start_s && t < seg.end_s);
    };
    player = makePlayer(session, duration, onTick);
    playButton.addEventListener("click", () => player?.toggle());
    // Clip starts, bookmarks and action items, for jumping to the previous or next in the timeline.
    const starts = [...new Set([...session.segments.map((s) => s.start_s), ...allMarkers.map((m) => m.at_s)])].sort((a, b) => a - b);
    const prevClip = () => {
      // Within a moment of a clip's start, go to the one before, as a music player does.
      const t = (player?.t ?? 0) - 0.5;
      player?.seek([...starts].reverse().find((s) => s < t) ?? 0);
    };
    const nextClip = () => {
      const t = (player?.t ?? 0) + 0.05;
      const next = starts.find((s) => s > t);
      if (next != null) player?.seek(next);
    };
    tracks.append(lanes, el("div", { class: "transport" },
      el("button", { class: "pill", "aria-label": "Previous clip or mark", title: "Start of the previous clip, bookmark or action item", onclick: prevClip }, svg("skip-back", 16)),
      el("button", { class: "pill", "aria-label": "Back 5 seconds", onclick: () => player?.seek(player.t - 5) }, "−5"),
      playButton,
      el("button", { class: "pill", "aria-label": "Forward 5 seconds", onclick: () => player?.seek(player.t + 5) }, "+5"),
      el("button", { class: "pill", "aria-label": "Next clip or mark", title: "Start of the next clip, bookmark or action item", onclick: nextClip }, svg("skip-forward", 16)),
      timeLabel, el("span", { class: "grow" }),
      el("span", { class: "hint windows-only" }, "Space plays and pauses")));
    requestAnimationFrame(() => player?.seek(keepTime));
  }
  main.replaceChildren(el("div", { class: "review" }, page, streams.length ? tracks : null));
}

/// A phrase heard again this close to a marker made while recording is the same moment.
const SAME_MARK_S = 5;
const PHRASE_KINDS = { bookmark: "", action: "action" };

/// The markers made while recording, and "Bookmark that" or "Action item" heard in the recording when it was
/// processed, as when nothing was listening for them at the time.
function sessionMarkers(saved, phrases) {
  const heard = phrases
    .filter((h) => !h.rejected && h.phrase in PHRASE_KINDS)
    .map((h) => ({ at_s: Math.round(h.start_s * 10) / 10, text: "", kind: PHRASE_KINDS[h.phrase], heard: true }))
    .filter((h) => !saved.some((m) => isAction(m) === isAction(h) && Math.abs(m.at_s - h.at_s) < SAME_MARK_S));
  return [...saved, ...heard].sort((a, b) => a.at_s - b.at_s);
}

/// What a marker says: the note typed with it, or for an action item, what was said where it was marked.
function markerNote(m, segments) {
  if (m.text && m.text !== "Action item") return m.text;
  return isAction(m) ? actionItemText(m, segments) : "";
}

/// The action an action item was about: the words after "action item" when it was said, otherwise the line
/// being spoken when it was marked, or the one just before.
function actionItemText(m, segments) {
  const lines = segments.filter((s) => s.text?.trim()).sort((a, b) => a.start_s - b.start_s);
  let i = lines.findIndex((s) => s.start_s <= m.at_s + 1 && s.end_s >= m.at_s - 1);
  if (i < 0) i = lines.findLastIndex((s) => s.end_s < m.at_s && m.at_s - s.end_s < 15);
  if (i < 0) return "";
  let text = lines[i].text.trim();
  const said = /\baction[\s-]*items?\b[\s,.:;-]*/i.exec(text);
  if (said) {
    text = text.slice(said.index + said[0].length).trim();
    // "Action item." on its own line: the action is in the next one.
    if (text.split(/\s+/).filter(Boolean).length < 3 && lines[i + 1]) text = `${text} ${lines[i + 1].text.trim()}`.trim();
  }
  return text.charAt(0).toUpperCase() + text.slice(1);
}

// Space plays and pauses a meeting when not typing.
window.addEventListener("keydown", (e) => {
  if (view.kind !== "session" || !player || e.code !== "Space" || e.ctrlKey) return;
  const t = e.target;
  if (t.isContentEditable || ["INPUT", "TEXTAREA", "SELECT", "BUTTON"].includes(t.tagName)) return;
  e.preventDefault();
  player.toggle();
});

/// The ⋯ menu beside a speaker: merge into another speaker, or link, unlink or create a voice print.
function speakerMenu(sp, speakers, onPick) {
  const others = speakers.filter((o) => o.key !== sp.key);
  const prints = (window.currentPrints ?? []).filter((p) => p.id !== sp.print?.id);
  const menu = el("select", { title: "More for this speaker", "aria-label": `More for ${sp.name}` },
    el("option", { value: "" }, "⋯"),
    others.length ? el("optgroup", { label: "Same person as" }, others.map((o) => el("option", { value: `merge:${o.key}` }, o.name))) : null,
    prints.length ? el("optgroup", { label: "This is voice print" }, prints.map((p) => el("option", { value: `link:${p.id}` }, p.name))) : null,
    el("optgroup", { label: "Voice print" },
      sp.voiced_s >= 8 ? el("option", { value: "new-print" }, "Save as a new voice print…") : null,
      sp.print ? el("option", { value: "unlink" }, `Not ${sp.print.name}`) : null,
      sp.voiced_s < 8 ? el("option", { disabled: true }, "Too little clear speech for a new voice print") : null));
  menu.addEventListener("change", () => {
    const value = menu.value;
    menu.value = "";
    if (value) onPick(value);
  });
  return menu;
}

/// An inline name box beside the dropdown, because the webview has no prompt dialog.
function askName(anchor) {
  return new Promise((resolve) => {
    const input = el("input", { placeholder: "Name", class: "chip" });
    anchor.after(input);
    input.focus();
    const done = (value) => { input.remove(); resolve(value); };
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") done(input.value.trim());
      if (e.key === "Escape") done("");
    });
    input.addEventListener("blur", () => done(input.value.trim()));
  });
}

// ---------- Voice prints ----------

/// Written to cover most English sounds in about 30 seconds of reading.
const BUILT_IN_SCRIPTS = [
  { title: "Morning walk", text: "On a cool morning, Jo packed a thick brown jacket, two cheese sandwiches and a flask of hot tea. "
    + "The path wound up through the woods, past a quiet pond where three white geese were splashing. "
    + "By noon the view from the ridge stretched for miles: yellow fields, a red barn, and a village with a tall church. "
    + "\"Should we keep going?\" asked Max. \"Sure,\" Jo said, \"but let's measure how much water we have left first.\"" },
  { title: "Work update", text: "Here is a quick update on the project. Last week we fixed the login problem, reviewed the budget "
    + "and agreed on a schedule with the supplier. This week the focus is testing: Zoe is checking the mobile version, "
    + "and Raj is measuring how fast the reports load. If anything blocks you, just ask. "
    + "Our next big date is the fourteenth of June, when the first customers join the pilot." },
];

let enrolment = null;

async function showVoicePrints(error) {
  leave();
  showPage();
  view = { kind: "prints" };
  refreshItems();
  const prints = await invoke("list_voiceprints");
  const page = el("div", { class: "page" },
    el("button", { class: "link", onclick: () => showSettings() }, "‹ Settings"),
    el("h1", {}, "Voice prints"),
    el("p", { class: "legend" }, "A voice print lets Superteem recognise someone and name them in new meetings. "
      + "It is kept only on this device, as a description of how the voice sounds; the recording made for it is deleted. "
      + "Only make one with the person's agreement."));
  if (error) page.append(el("div", { class: "error" }, String(error)));
  if (!prints.length) page.append(el("div", { class: "empty" }, "No voice prints yet."));
  for (const p of prints) {
    const name = el("input", { value: p.name, title: "Rename", "aria-label": "Name" });
    name.addEventListener("keydown", (e) => { if (e.key === "Enter") name.blur(); });
    name.addEventListener("change", async () => {
      try { await invoke("rename_voiceprint", { id: p.id, name: name.value }); } catch (e) { showError(page, e); }
    });
    const remove = el("button", { class: "link danger" }, "Delete");
    remove.addEventListener("click", async () => {
      if (!remove.dataset.armed) {
        remove.dataset.armed = "1";
        remove.textContent = "Click again to delete";
        setTimeout(() => { delete remove.dataset.armed; remove.textContent = "Delete"; }, 4000);
        return;
      }
      try { await invoke("delete_voiceprint", { id: p.id }); showVoicePrints(); } catch (e) { showError(page, e); }
    });
    const sources = [p.scripts ? `${p.scripts} script reading${p.scripts > 1 ? "s" : ""}` : null,
      p.samples - p.scripts ? `${p.samples - p.scripts} meeting${p.samples - p.scripts > 1 ? "s" : ""}` : null].filter(Boolean);
    page.append(el("div", { class: "print-row" }, name,
      el("span", { class: "detail" }, `${Math.round(p.seconds)} s of speech from ${sources.join(" and ")}`),
      el("button", { onclick: () => showEnrol(p) }, "Record more"), remove));
  }
  page.append(el("div", { class: "actions" }, el("button", { class: "primary pill", onclick: () => showEnrol(null) }, "New voice print")),
    el("p", { class: "legend" }, "You can also make a voice print from a speaker in any meeting, from the ⋯ menu beside their name."));
  main.replaceChildren(page);
}

/// Sets up a script reading for a new voice print, or more speech for `print`.
async function showEnrol(print) {
  leave();
  showPage();
  view = { kind: "prints" };
  const [{ devices, default: preferred }, fresh] = await Promise.all([invoke("list_devices"), invoke("get_prefs")]);
  prefs = fresh;
  const page = el("div", { class: "page" }, el("h1", {}, print ? `Record more of ${print.name}` : "New voice print"));
  const name = el("input", { type: "text", class: "plain", placeholder: "Their name", "aria-label": "Their name" });
  if (!print) page.append(el("h2", {}, "Name"), name);

  page.append(el("h2", {}, "Microphone"));
  const input = el("select", { class: "plain", "aria-label": "Microphone" }, devices.map((d) => el("option", { value: d,
    selected: d === (devices.includes(prefs.enrol_input) ? prefs.enrol_input : preferred) }, d)));
  const micInfo = el("span");
  const showNote = () => micInfo.replaceChildren(micNote(prefs.mics[input.value]) ?? "");
  input.addEventListener("change", showNote);
  showNote();
  page.append(el("div", { class: "field" }, input, micInfo),
    el("div", { class: "legend" }, "Use the microphone they will usually wear, so they are recognised best in their meetings."));

  page.append(el("h2", {}, "Script"));
  const scripts = [...BUILT_IN_SCRIPTS, ...prefs.scripts.map((s) => ({ ...s, custom: true }))];
  const choice = el("select", { class: "plain", "aria-label": "Script" }, scripts.map((s, i) => el("option", { value: i }, s.title + (s.custom ? " (yours)" : ""))),
    el("option", { value: "new" }, "Write your own…"));
  const preview = el("div", { class: "script" });
  const editor = el("div");
  const showScript = () => {
    editor.replaceChildren();
    if (choice.value === "new") {
      preview.style.display = "none";
      const title = el("input", { type: "text", placeholder: "Script name" });
      const text = el("textarea", { class: "plain", placeholder: "Something they can read aloud for 30 seconds or more" });
      const saveScript = el("button", { class: "primary" }, "Save script");
      saveScript.addEventListener("click", async () => {
        if (!title.value.trim() || !text.value.trim()) return showError(editor, "Give the script a name and some text.");
        await invoke("save_prefs", { scripts: [...prefs.scripts, { title: title.value.trim(), text: text.value.trim() }] });
        showEnrol(print);
      });
      editor.append(el("div", { class: "inline-form" }, title, text, saveScript));
      return;
    }
    const s = scripts[choice.value];
    preview.style.display = "";
    preview.textContent = s.text;
    if (s.custom) {
      const drop = el("button", { class: "link danger" }, "Delete this script");
      drop.addEventListener("click", async () => {
        await invoke("save_prefs", { scripts: prefs.scripts.filter((x) => x.title !== s.title || x.text !== s.text) });
        showEnrol(print);
      });
      editor.append(drop);
    }
  };
  choice.addEventListener("change", showScript);
  page.append(choice, preview, editor);
  showScript();

  page.append(el("h2", {}, "Consent"));
  const consent = el("input", { type: "checkbox" });
  page.append(el("label", { class: "field" }, consent, "This person agreed to have a voice print kept on this device"));
  const start = el("button", { class: "primary pill" }, "Start reading");
  start.addEventListener("click", async () => {
    if (choice.value === "new") return showError(page, "Save the script first.");
    try {
      await invoke("start_enrolment", { device: input.value, name: name.value, print: print?.id ?? null, consent: consent.checked });
      enrolment = { print, name: print?.name ?? name.value.trim(), script: scripts[choice.value].text };
      showEnrolRecording();
    } catch (e) {
      showError(page, e);
    }
  });
  page.append(el("div", { class: "actions" }, start, el("button", { class: "pill", onclick: () => showVoicePrints() }, "Cancel")));
  if (!devices.length) start.disabled = true;
  main.replaceChildren(page);
}

function showEnrolRecording() {
  clearInterval(pollTimer);
  view = { kind: "prints" };
  const banner = el("div", { class: "recording-banner" });
  const meter = el("div", { style: "width:0%" });
  const stop = el("button", { class: "primary pill" }, "Stop and save");
  const cancel = el("button", { class: "pill" }, "Cancel");
  const page = el("div", { class: "page" }, el("h1", {}, `Reading for ${enrolment.name}`),
    el("p", { class: "legend" }, "Read at a normal pace and volume. About 30 seconds is ideal; at least 8 seconds of speech is needed."),
    banner, el("div", { class: "meter wide-meter" }, meter), el("div", { class: "script" }, enrolment.script),
    el("div", { class: "actions" }, stop, cancel));
  main.replaceChildren(page);
  const poll = async () => {
    const s = await invoke("recording_status");
    if (!s.recording) return;
    banner.textContent = `● RECORDING  ${clock(s.elapsed_s)}`;
    meter.style.width = `${Math.round((s.streams[0]?.level ?? 0) * 100)}%`;
  };
  poll();
  pollTimer = setInterval(poll, 250);
  const finish = async (save) => {
    clearInterval(pollTimer);
    stop.disabled = cancel.disabled = true;
    try {
      await invoke("stop_enrolment", { save });
      enrolment = null;
      showVoicePrints();
    } catch (e) {
      enrolment = null;
      showVoicePrints(e);
    }
  };
  stop.addEventListener("click", () => finish(true));
  cancel.addEventListener("click", () => finish(false));
}

// ---------- Wake word and commands ----------

/// Set while the training page is open: it shows phrases as they are heard instead of acting on them.
let keyphrasePage = null;
/// Set while takes of a phrase are being recorded.
let phraseTraining = null;

/// How well a phrase stands apart from the person's other phrases and from the same sounds backwards.
function phraseQuality(separation) {
  if (separation == null) return "needs at least 2 takes";
  if (separation >= 1.6) return "clear";
  if (separation >= 1.3) return "fair";
  return "easily confused: record it again, more clearly";
}

/// The voice print the phrases are trained for: the one chosen, or the first made, which is usually the
/// person who set up the app.
async function keyphrasePrint(id) {
  const prints = await invoke("list_voiceprints");
  const first = [...prints].sort((a, b) => a.created - b.created)[0];
  return { prints, print: prints.find((p) => p.id === id) ?? first };
}

async function showKeyphrases(printId, message) {
  leave();
  // Phrases heard while the page loads, such as the last take, are shown here rather than acted on.
  keyphrasePage = { trying: false, onHeard() {}, onWords() {}, stop() {} };
  showPage();
  view = { kind: "keyphrases" };
  refreshItems();
  const [{ prints, print }, fresh] = await Promise.all([keyphrasePrint(printId), invoke("get_prefs")]);
  prefs = fresh;
  const page = el("div", { class: "page" },
    el("button", { class: "link", onclick: () => showSettings() }, "‹ Settings"),
    el("h1", {}, "Wake word and commands"),
    el("p", { class: "legend" }, "Say each phrase a few times so Superteem hears it the moment it's said, without waiting "
      + "for speech to text. It learns how you say them, so it answers you rather than anyone in the room. Only a "
      + "description of the sound is kept with your voice print; the recordings are deleted."));
  if (!print) {
    page.append(el("div", { class: "notice" }, "Make your voice print first: the phrases are kept with it."),
      el("div", { class: "actions" }, el("button", { class: "primary pill", onclick: () => showOnboarding(0) }, "Make my voice print")));
    main.replaceChildren(page);
    return;
  }
  if (message) page.append(el("div", { class: "notice", role: "status" }, message));
  if (prints.length > 1) {
    const who = el("select", { class: "plain", "aria-label": "Whose phrases" },
      prints.map((p) => el("option", { value: p.id, selected: p.id === print.id }, p.name)));
    who.addEventListener("change", () => showKeyphrases(who.value));
    page.append(el("div", { class: "field" }, "Phrases for", who));
  }
  const kp = await invoke("list_keyphrases", { print: print.id });
  phrasesTrained = kp.any_trained;
  const device = prefs.enrol_input;
  const record = async (phrase, replace) => {
    try {
      await invoke("start_phrase_training", { print: print.id, phrase: phrase.id, device, replace });
      phraseTraining = { print, phrase };
      showPhraseTakes();
    } catch (e) {
      showError(page, e);
    }
  };
  const group = el("div", { class: "group" });
  for (const phrase of kp.phrases) {
    const status = phrase.takes
      ? `${phrase.takes} take${phrase.takes > 1 ? "s" : ""} · ${phraseQuality(phrase.separation)}`
      : "Not trained yet";
    const actions = phrase.takes
      ? [el("button", { class: "pill", onclick: () => record(phrase, true) }, "Record again"),
        phrase.takes < 12 ? el("button", { class: "pill", onclick: () => record(phrase, false) }, "Add takes") : null,
        el("button", { class: "link danger", onclick: async () => {
          try { await invoke("clear_keyphrase", { print: print.id, phrase: phrase.id }); showKeyphrases(print.id); } catch (e) { showError(page, e); }
        } }, "Clear")]
      : [el("button", { class: "primary pill", onclick: () => record(phrase, true) }, "Train")];
    group.append(el("div", { class: "row", id: `phrase-${phrase.id}` },
      el("span", { class: "label" }, `“${phrase.text}”`, el("small", {}, phrase.does), el("small", {}, status)),
      el("span", { class: "phrase-actions" }, actions)));
  }
  page.append(el("h2", {}, "Phrases"), group);
  if (!device) page.append(el("div", { class: "error" }, "Set up your default microphone first, from Settings."));

  // Try it: press, then speak. Each phrase heard says which command it is, and listening stops after
  // 10 seconds with nothing heard.
  const TRY_S = 10;
  const tryButton = el("button", { class: "primary pill", type: "button" }, "Try it");
  const heard = el("div", { class: "wake-indicator", "aria-live": "polite" },
    kp.any_trained ? "Press Try it, then say a phrase." : "Train a phrase above, then try it here.");
  tryButton.disabled = !kp.any_trained;
  const results = el("ul", { class: "try-results" });
  let quiet = null;
  const show = (text, understood) => {
    heard.textContent = text;
    heard.classList.add("on");
    if (understood) results.prepend(el("li", {}, understood));
    while (results.children.length > 5) results.lastChild.remove();
  };
  const stopTrying = (text) => {
    if (!keyphrasePage?.trying) return;
    clearTimeout(quiet);
    keyphrasePage.trying = false;
    tryButton.textContent = "Try it";
    heard.classList.remove("on");
    if (text) heard.textContent = text;
    syncWake().catch(() => {});
  };
  const keepListening = () => {
    clearTimeout(quiet);
    quiet = setTimeout(() => stopTrying(`Stopped listening: nothing was heard for ${TRY_S} seconds. Press Try it again.`), TRY_S * 1000);
  };
  const commandName = (id) => COMMANDS.find((c) => c.id === id)?.say;
  keyphrasePage = {
    trying: false,
    stop: () => stopTrying(),
    // A trained phrase, heard by the spotter.
    onHeard(h) {
      if (phraseTraining || !this.trying) return;
      keepListening();
      const timing = `heard ${Math.round(h.delay_ms)} ms after you finished · score ${h.score.toFixed(2)}`;
      if (h.rejected) return show(`“${h.text}” didn't sound like ${h.who || "you"}, so it would be ignored · ${timing}`);
      if (h.phrase === "wake") {
        chime();
        return show(`“Hey Superteem” · ${timing}. Now say a command.`);
      }
      const command = commandName(KEYPHRASE_COMMANDS[h.phrase]);
      show(`Understood: ${command} · ${timing}`, `“${h.text}” → ${command}`);
    },
    // Words from speech to text: a command after "Hey Superteem", or a phrase not trained yet.
    onWords(text) {
      if (!this.trying) return;
      keepListening();
      const wake = WAKE.exec(text);
      const rest = (wake ? text.slice(wake.index + wake[0].length) : text).trim();
      const [best] = rest ? interpret(rest) : [];
      if (best?.score >= 1) return show(`Understood: ${best.say} · from the words “${text}”`, `“${text}” → ${best.say}`);
      show(rest ? `Heard “${text}”, which isn't a command.` : `Heard “${text}”. Now say a command.`);
    },
  };
  tryButton.addEventListener("click", async () => {
    if (keyphrasePage.trying) return stopTrying("Stopped listening.");
    keyphrasePage.trying = true;
    try {
      await syncWake();
    } catch (e) {
      keyphrasePage.trying = false;
      return showError(page, e);
    }
    tryButton.textContent = "Stop";
    show(`Listening… say a trained phrase, or “Hey Superteem” and a command. Stops after ${TRY_S} quiet seconds.`);
    keepListening();
  });

  const sensitivity = el("input", { type: "range", min: "0.7", max: "1.4", step: "0.05", value: kp.sensitivity, "aria-label": "Sensitivity" });
  const saveSettings = async () => {
    try {
      await invoke("save_keyphrase_settings", { sensitivity: Number(sensitivity.value), verify: kp.verify });
    } catch (e) {
      showError(page, e);
    }
  };
  sensitivity.addEventListener("change", saveSettings);
  page.append(el("h2", {}, "Try it"), el("div", { class: "try-it" }, tryButton, heard), results,
    el("div", { class: "group" },
      el("div", { class: "row" }, el("span", { class: "label" }, "Sensitivity",
        el("small", {}, "If phrases are missed, slide right or add takes. If it wakes when nobody said it, slide left.")),
        el("span", { class: "slider" }, "Stricter", sensitivity, "Hears more")),
      toggleRow("Only answer my voice", "Ignore a phrase when it doesn't also sound like the voice print it was trained by.", kp.verify, async (on) => {
        kp.verify = on;
        await saveSettings();
      })));
  main.replaceChildren(page);
}

/// Recording someone saying one phrase several times.
function showPhraseTakes() {
  leave();
  view = { kind: "keyphrases" };
  const { print, phrase } = phraseTraining;
  const banner = el("div", { class: "recording-banner" });
  const meter = el("div", { style: "width:0%" });
  const stop = el("button", { class: "primary pill" }, "Done");
  const cancel = el("button", { class: "pill" }, "Cancel");
  const page = el("div", { class: "page" }, el("h1", {}, `Say “${phrase.text}”`),
    el("p", { class: "legend" }, "Say it once for each line, with a pause of about a second between. Say it a little "
      + "differently each time: that teaches Superteem the ways you really say it."
      + (phrase.after ? ` Say just “${phrase.text}”; it only counts after “${phrase.after}”.` : "")),
    banner, el("div", { class: "meter wide-meter" }, meter),
    el("ol", { class: "takes" }, phrase.how.map((h) => el("li", {}, h))),
    el("div", { class: "actions" }, stop, cancel));
  main.replaceChildren(page);
  const poll = async () => {
    const s = await invoke("recording_status");
    if (!s.recording) return;
    banner.textContent = `● RECORDING  ${clock(s.elapsed_s)}`;
    meter.style.width = `${Math.round((s.streams[0]?.level ?? 0) * 100)}%`;
  };
  poll();
  pollTimer = setInterval(poll, 250);
  const finish = async (save) => {
    clearInterval(pollTimer);
    stop.disabled = cancel.disabled = true;
    stop.textContent = save ? "Learning the takes…" : "Done";
    try {
      const r = await invoke("stop_phrase_training", { save }).finally(() => (phraseTraining = null));
      if (!r) return showKeyphrases(print.id);
      const few = r.heard < phrase.how.length - 1
        ? " That's fewer than expected: leave a clear pause between takes, or use Add takes." : "";
      showKeyphrases(print.id, `Heard ${r.heard} take${r.heard === 1 ? "" : "s"} of “${phrase.text}”; ${r.takes} kept. `
        + `It is ${phraseQuality(r.separation)}.${few}`);
    } catch (e) {
      showKeyphrases(print.id, String(e));
    }
  };
  stop.addEventListener("click", () => finish(true));
  cancel.addEventListener("click", () => finish(false));
}

// ---------- Settings ----------

function toggleRow(label, detail, checked, onChange) {
  const box = el("input", { type: "checkbox", class: "switch", checked, "aria-label": label });
  box.addEventListener("change", () => onChange(box.checked, box));
  return el("label", { class: "row" }, el("span", { class: "label" }, label, detail ? el("small", {}, detail) : null), box);
}

/// How many of the first voice print's phrases are trained, for the settings row.
function trainedText(prints) {
  const first = [...prints].sort((a, b) => a.created - b.created)[0];
  if (!first) return "Needs a voice print";
  return first.phrases ? `${first.phrases} of ${first.phrases_of} trained` : "Not trained: slower to answer";
}

function linkRow(label, value, onClick) {
  return el("button", { class: "row", style: "width:100%;border-radius:0;border:none;border-bottom:1px solid var(--line);text-align:left", onclick: onClick },
    el("span", { class: "label" }, label), el("span", { class: "value" }, value ?? ""), el("span", { "aria-hidden": "true", class: "value" }, "›"));
}

async function showSettings() {
  leave();
  showPage();
  view = { kind: "settings" };
  refreshItems();
  const [fresh, ready, prints] = await Promise.all([invoke("get_prefs"), invoke("model_ready"), invoke("list_voiceprints").catch(() => [])]);
  prefs = fresh;
  const page = el("div", { class: "page" }, el("h1", {}, "Settings"));

  const model = el("div", { class: "row" }, el("span", { class: "label" }, "Speech model",
    el("small", {}, "Used for dictation, voice commands and transcripts. Runs on this device. English only.")));
  if (ready) model.append(el("span", { class: "value", style: "color:var(--ok)" }, "Ready"));
  else model.append(modelDownloadButton(() => showSettings()));

  page.append(el("h2", {}, "Listening"), el("div", { class: "group windows-only" },
    toggleRow("“Hey Superteem”", "Listen for the wake word while the app is open. Nothing is recorded until you ask.", prefs.wake,
      async (on, box) => {
        try {
          await setWake(on);
        } catch (e) {
          box.checked = false;
          showError(page, e);
        }
      }),
    toggleRow("Speak replies aloud", "So the app can be used without looking at the screen.", !prefs.quiet, async (on) => {
      prefs.quiet = !on;
      await invoke("save_prefs", { quiet: !on });
    }),
    toggleRow("Keep dictation audio", "Keep the recording of each note dictation for 1 week. When off, the audio is deleted once the words are written.", prefs.keep_audio, async (on) => {
      prefs.keep_audio = on;
      await invoke("save_prefs", { keepAudio: on });
    }),
    model));
  if (platform === "android") page.append(el("div", { class: "group" }, model));

  page.append(el("h2", {}, "Voices"), el("div", { class: "group" },
    linkRow("Voice prints", prints.length ? `${prints.length} saved` : "None yet", () => showVoicePrints()),
    linkRow("Default microphone and voice print", prefs.enrol_input ?? "Not set up", () => showOnboarding(0)),
    el("div", { class: "windows-only" }, linkRow("Devices", "Test, hide and add microphones and Voice Pins", () => showDevices())),
    el("div", { class: "windows-only" }, linkRow("Wake word and commands", trainedText(prints), () => showKeyphrases()))));

  page.append(el("h2", {}, "Meetings"), el("div", { class: "group" },
    el("div", { class: "row" }, el("span", { class: "label" }, "Consent question",
      el("small", {}, "Superteem always confirms everyone agreed before a meeting is recorded.")), el("span", { class: "value" }, "Always asked")),
    el("div", { class: "row" }, el("span", { class: "label" }, "Tracks used last time"),
      el("span", { class: "value" }, [...prefs.inputs, prefs.pins ? "Voice Pins" : null].filter(Boolean).join(", ") || "Default microphone"))));

  const theme = el("div", { class: "theme", role: "group", "aria-label": "Theme" });
  const current = document.documentElement.dataset.theme ?? "system";
  for (const [value, label] of [["system", "System"], ["light", "Light"], ["dark", "Dark"]]) {
    theme.append(el("button", { class: current === value ? "on" : "", "aria-pressed": String(current === value),
      onclick: () => { pickTheme(value); showSettings(); } }, label));
  }
  page.append(el("h2", {}, "Appearance"), el("div", { class: "group" }, el("div", { class: "row" }, el("span", { class: "label" }, "Theme"), theme)));

  page.append(el("h2", {}, "Storage"), el("div", { class: "group" },
    el("div", { class: "row" }, el("span", { class: "label" }, "Recordings and notes", el("small", {}, "Kept on this device only.")),
      platform === "windows" ? el("button", { class: "pill", onclick: () => invoke("open_folder", { id: null }) }, "Open folder") : null)));
  main.replaceChildren(page);
}

function modelDownloadButton(done) {
  const holder = el("span");
  const button = el("button", { class: "primary pill" }, "Download (142 MB)");
  button.addEventListener("click", async () => {
    button.disabled = true;
    const label = el("span", {}, "Downloading…");
    button.replaceChildren(el("span", { class: "spinner" }), label);
    const unlisten = await listen("model-download", (e) => {
      label.textContent = `Downloading… ${Math.round(e.payload * 100)}%`;
    });
    try {
      await invoke("download_model");
      unlisten();
      done();
    } catch (e) {
      unlisten();
      button.disabled = false;
      button.textContent = "Try the download again";
      showError(holder, e);
    }
  });
  holder.append(button);
  return holder;
}

async function setWake(on) {
  await invoke("set_wake", { on });
  prefs.wake = on;
  await invoke("save_prefs", { wake: on });
  showListening();
}

function showListening() {
  const on = prefs.wake;
  document.getElementById("listen-chip").classList.toggle("on", on);
  document.getElementById("listen-label").textContent = on ? "Listening" : "Voice off";
}

// ---------- First run ----------

let onboardEnrolling = false;

/// Cancels a voice print reading left half done, so the microphone is released.
function cancelOnboardReading() {
  if (!onboardEnrolling) return;
  onboardEnrolling = false;
  invoke("stop_enrolment", { save: false }).catch(() => {});
}

async function showOnboarding(step = 0) {
  leave();
  showPage();
  view = { kind: "onboarding", step };
  const ready = await invoke("model_ready");
  const count = platform === "android" ? 3 : 4;
  const shown = platform === "android" && step === 3 ? 2 : step;
  const steps = el("div", { class: "steps", "aria-label": `Step ${shown + 1} of ${count}` },
    Array.from({ length: count }, (_, i) => el("span", { class: i < shown ? "done" : i === shown ? "now" : "" })));
  const page = el("div", { class: "page" }, steps);
  const finish = async () => {
    cancelOnboardReading();
    await invoke("save_prefs", { onboarded: true }).catch(() => {});
    prefs.onboarded = true;
    showHome();
  };
  const afterVoice = () => showOnboarding(1);
  if (step === 0) {
    page.append(el("h1", {}, "Welcome to Superteem"),
      el("p", {}, "Write notes by voice, record meetings with a track for every voice, or just say “Hey Superteem”. First, let's get your microphone working and make your voice print."),
      el("p", { class: "legend" }, "Pick the microphone you'll usually use, then read a short passage. It becomes your default microphone for dictation, voice commands and meetings, and Superteem names you in meetings."));
    const body = el("div");
    page.append(body, el("div", { class: "actions" },
      el("button", { class: "link", onclick: () => { cancelOnboardReading(); afterVoice(); } }, "Skip this step"),
      el("button", { class: "link", onclick: finish }, "Skip setup")));
    main.replaceChildren(page);
    return voiceSetup(body, afterVoice);
  } else if (step === 1) {
    page.append(el("h1", {}, "The speech model"),
      el("p", {}, "Everything you say is turned into text on this device, never sent anywhere. That needs a one-time download."),
      ready ? el("p", { style: "color:var(--ok)" }, "Ready.") : modelDownloadButton(() => showOnboarding(1)),
      el("div", { class: "actions" },
        el("button", { class: "primary pill", onclick: () => showOnboarding(platform === "android" ? 3 : 2) }, "Continue")));
  } else if (step === 2) {
    const tick = el("div", { class: "ticks", id: "onboard-ticks" }, el("span"));
    const state = el("div", { class: "hint", id: "onboard-state", "aria-live": "polite", style: "text-align:center" },
      ready ? "Listening… say “Hey Superteem”" : "The wake word needs the speech model. Go back a step to get it, or skip.");
    page.append(el("h1", {}, "Say hello"),
      el("p", {}, "Say “Hey Superteem”. From now on Superteem listens for it while the app is open, so you can use it without the screen. Nothing is recorded until you ask."),
      el("div", { class: "halo" }, el("span", { class: "big-mic", style: "width:104px;height:104px" }, svg("mic", 40))),
      tick, state,
      el("p", { class: "legend", style: "text-align:center" }, "Train it to your voice and it answers in a tenth of a second, instead of waiting for speech to text."),
      el("div", { class: "actions", style: "justify-content:center" },
        el("button", { class: "pill", onclick: () => showKeyphrases() }, "Train it to my voice"),
        el("button", { class: "primary pill", onclick: () => showOnboarding(3) }, "Continue"),
        el("button", { class: "pill", onclick: async () => { await setWake(false).catch(() => {}); showOnboarding(3); } }, "Don't listen for it")));
    if (ready) setWake(true).catch((e) => { state.textContent = String(e); });
  } else {
    page.append(el("h1", {}, "You're set"),
      el("div", { class: "group" },
        el("div", { class: "row" }, el("span", { class: "label" }, "Write a note", el("small", {}, "Type anywhere. Press Dictate, or Ctrl + Space, to speak up to 2 minutes. Keep typing while you talk. The tick writes the words at the cursor, the cross throws them away."))),
        el("div", { class: "row" }, el("span", { class: "label" }, "Record a meeting", el("small", {}, "Each microphone gets its own track, and each voice its own lane."))),
        el("div", { class: "row windows-only" }, el("span", { class: "label" }, "Talk to it", el("small", {}, "“Hey Superteem, start meeting.” “Take a note.” “What can I say?”")))),
      el("div", { class: "actions" }, el("button", { class: "primary pill", onclick: finish }, "Start")));
  }
  main.replaceChildren(page);
}

/// Seconds of speech a good voice print needs, and the least that is accepted.
const VOICE_TARGET_S = 20;
const VOICE_MIN_S = 8;

/// Chooses a microphone, checks it hears speech, then records a script reading into the person's voice print.
async function voiceSetup(holder, next, problem) {
  const [{ devices, default: preferred }, fresh] = await Promise.all([invoke("list_devices"), invoke("get_prefs")]);
  prefs = fresh;
  const name = el("input", { type: "text", class: "plain", placeholder: "Your name", "aria-label": "Your name", value: holder.dataset.name ?? "" });
  const mic = el("select", { class: "plain", "aria-label": "Microphone" }, devices.map((d) => el("option", { value: d,
    selected: d === (holder.dataset.mic ?? (devices.includes(prefs.enrol_input) ? prefs.enrol_input : preferred)) }, d)));
  const micInfo = el("span");
  const showNote = () => micInfo.replaceChildren(micNote(prefs.mics[mic.value]) ?? "");
  mic.addEventListener("change", showNote);
  showNote();
  const consent = el("input", { type: "checkbox" });
  const start = el("button", { class: "primary pill" }, "Check microphone and start");
  holder.replaceChildren(
    problem ? el("div", { class: "notice", role: "alert" }, problem) : null,
    el("h2", {}, "Your name"), name,
    el("h2", {}, "Microphone"), el("div", { class: "field" }, mic, micInfo),
    devices.length ? null : el("div", { class: "error" }, "No microphones found. Connect one and come back to this step."),
    el("label", { class: "field" }, consent, "Keep my voice print on this device"),
    el("div", { class: "legend" }, "Only a description of how your voice sounds is kept. The recording is deleted."),
    el("div", { class: "actions" }, start));
  start.disabled = !devices.length;
  start.addEventListener("click", async () => {
    if (!name.value.trim()) return showError(holder, "Type your name, so meetings can show it.");
    if (!consent.checked) return showError(holder, "Tick the box to agree to keeping your voice print.");
    holder.dataset.name = name.value.trim();
    holder.dataset.mic = mic.value;
    try {
      await invoke("start_enrolment", { device: mic.value, name: name.value.trim(), print: null, consent: true });
    } catch (e) {
      return showError(holder, e);
    }
    onboardEnrolling = true;
    voiceReading(holder, next, mic.value);
  });
}

function voiceReading(holder, next, device) {
  const status = el("div", { class: "hint", "aria-live": "polite" }, `Say something, so we can hear ${device}.`);
  const meter = el("div", { style: "width:0%" });
  const progressBar = el("div", { style: "width:0%" });
  const script = el("div", { class: "script", hidden: true }, BUILT_IN_SCRIPTS[1].text);
  const reading = el("div", { hidden: true }, el("div", { class: "legend" }, "Speech so far"), el("div", { class: "meter wide-meter" }, progressBar));
  const done = el("button", { class: "primary pill", disabled: true }, "Done");
  const other = el("button", { class: "pill" }, "Use a different microphone");
  const heading = el("h2", {}, "Checking the microphone");
  holder.replaceChildren(heading, status,
    el("div", { class: "meter wide-meter", "aria-hidden": "true" }, meter), script, reading, el("div", { class: "actions" }, done, other));

  let heardFrames = 0;
  let quietSince = Date.now();
  let speech = 0;
  let checked = false;
  const stopReading = async (save) => {
    clearInterval(pollTimer);
    done.disabled = other.disabled = true;
    onboardEnrolling = false;
    return invoke("stop_enrolment", { save });
  };
  other.addEventListener("click", async () => {
    await stopReading(false).catch(() => {});
    voiceSetup(holder, next);
  });
  done.addEventListener("click", async () => {
    status.replaceChildren(el("span", { class: "spinner" }), "Making your voice print…");
    try {
      await stopReading(true);
      speak("Got it. I'll know your voice now.");
      holder.replaceChildren(el("div", { class: "consent-card" },
        el("strong", { style: "color:var(--ok)" }, "Microphone working, voice print saved."),
        el("div", { class: "legend" }, `${device} is now your default microphone for dictation, voice commands and meetings.`)),
        el("div", { class: "actions" }, el("button", { class: "primary pill", onclick: next }, "Continue")));
    } catch (e) {
      voiceSetup(holder, next, `The voice print wasn't saved: ${e}`);
    }
  });

  const poll = async () => {
    const s = await invoke("recording_status").catch(() => null);
    const stream = s?.streams?.[0];
    if (!s?.recording || !stream || !onboardEnrolling) return;
    if (stream.state === "failed") {
      await stopReading(false).catch(() => {});
      return voiceSetup(holder, next, `${device} could not be used: ${stream.detail || "it did not start"}. Try another microphone.`);
    }
    meter.style.width = `${Math.round(stream.level * 100)}%`;
    const loud = stream.level > 0.35;
    if (!checked) {
      heardFrames = loud ? heardFrames + 1 : 0;
      if (loud) quietSince = Date.now();
      if (heardFrames >= 3) {
        checked = true;
        heading.textContent = "Making your voice print";
        status.textContent = "Microphone working. Now read this aloud at your normal pace:";
        script.hidden = false;
        reading.hidden = false;
      } else if (Date.now() - quietSince > 6000) {
        status.replaceChildren(el("span", { class: "danger" }, `We can't hear anything from ${device}. Check it's plugged in and not muted, or use a different microphone.`));
      }
      return;
    }
    if (loud) speech += 0.2;
    progressBar.style.width = `${Math.min(100, (100 * speech) / VOICE_TARGET_S)}%`;
    done.disabled = speech < VOICE_MIN_S;
    if (speech >= VOICE_TARGET_S) status.textContent = "That's plenty. Press Done.";
  };
  clearInterval(pollTimer);
  pollTimer = setInterval(poll, 200);
}

function onboardTick() {
  const tick = document.querySelector("#onboard-ticks span");
  if (!tick || tick.classList.contains("done")) return;
  tick.classList.add("done");
  tick.replaceChildren(svg("check", 18));
  const state = document.getElementById("onboard-state");
  if (state) state.textContent = "Heard you. Superteem will answer to “Hey Superteem”.";
  speak("Hi. Say Hey Superteem whenever you need me.");
}

// ---------- The voice agent ----------

const agentEl = document.getElementById("agent");
const agentLog = document.getElementById("agent-log");
const agentState = document.getElementById("agent-state");
const agentActions = document.getElementById("agent-actions");
const agent = {
  open: false,
  mode: "idle", // idle | confirm
  pending: null, // the command a "Did you mean…?" asks about
  forcedWake: false,
  levelTimer: null,
};
let speaking = false;
/// Whether any phrase is trained, so the microphone can listen through a recording without transcribing
/// everything said in it.
let phrasesTrained = false;

/// Commands are listened for when voice is on, and also while the voice panel needs them, while phrases are
/// tried in Settings, and, once phrases are trained, through a meeting or a dictation, so
/// "Hey Superteem, stop recording" works whenever the app is recording.
function wantWake() {
  return prefs.wake || agent.forcedWake || !!keyphrasePage?.trying
    || (phrasesTrained && (!!recording || !!dictation));
}

function syncWake() {
  return platform === "android" ? Promise.resolve() : invoke("set_wake", { on: wantWake() });
}

/// Whether a meeting recording hears voice commands.
function meetingListening() {
  return platform !== "android" && (prefs.wake || phrasesTrained);
}

async function refreshPhrasesTrained() {
  phrasesTrained = (await invoke("list_keyphrases", { print: null }).catch(() => null))?.any_trained ?? phrasesTrained;
}

/// The voice panel steps back soon after a command in a meeting, so the meeting isn't taken as commands.
function stepBackSoon() {
  clearTimeout(agent.quietTimer);
  agent.quietTimer = setTimeout(() => { if (agent.open && agent.mode === "idle") closeAgent(); }, 2500);
}

function speak(text) {
  // The app's voice would be on the recording, so in a meeting it answers on screen only.
  if (prefs.quiet || !window.speechSynthesis || recording) return;
  speechSynthesis.cancel();
  const utterance = new SpeechSynthesisUtterance(text.replace(/[“”]/g, ""));
  speaking = true;
  invoke("listen_mute", { muted: true }).catch(() => {});
  utterance.onend = utterance.onerror = () => setTimeout(() => {
    speaking = false;
    invoke("listen_mute", { muted: false }).catch(() => {});
  }, 300);
  speechSynthesis.speak(utterance);
}

/// While the panel waits for an answer everything said is transcribed; otherwise only what follows "Hey Superteem".
function syncAttentive() {
  invoke("listen_attentive", { on: agent.open }).catch(() => {});
}

/// A short rising tone when the wake word is heard, so the person can carry straight on with the command.
let chimeAudio = null;
function chime() {
  try {
    chimeAudio ??= new AudioContext();
    const t = chimeAudio.currentTime;
    for (const [i, hz] of [660, 880].entries()) {
      const osc = chimeAudio.createOscillator();
      const gain = chimeAudio.createGain();
      osc.frequency.value = hz;
      gain.gain.setValueAtTime(0.0001, t + i * 0.09);
      gain.gain.exponentialRampToValueAtTime(0.12, t + i * 0.09 + 0.02);
      gain.gain.exponentialRampToValueAtTime(0.0001, t + i * 0.09 + 0.16);
      osc.connect(gain).connect(chimeAudio.destination);
      osc.start(t + i * 0.09);
      osc.stop(t + i * 0.09 + 0.18);
    }
  } catch {}
}

function say(text, big = false) {
  agentLog.append(el("div", { class: big ? "bot big" : "bot" }, text));
  agentLog.scrollTop = agentLog.scrollHeight;
  speak(text);
}

function heardLine(text) {
  agentLog.append(el("div", { class: "you" }, text));
  agentLog.scrollTop = agentLog.scrollHeight;
}

function agentButtons(...buttons) {
  agentActions.replaceChildren(...buttons.map(([label, action, primary]) =>
    el("button", { class: primary ? "primary" : "ghost", onclick: action }, label)));
}

function setAgentState(text) {
  agentState.textContent = text;
}

async function openAgent(greet = true) {
  if (!agent.open) {
    agent.open = true;
    agentEl.hidden = false;
    agentLog.replaceChildren();
    if (!prefs.wake && platform !== "android") {
      try {
        await invoke("set_wake", { on: true });
        agent.forcedWake = true;
      } catch (e) {
        say(String(e));
      }
    }
    const bars = [...agentEl.querySelectorAll(".bars span")];
    agent.levelTimer = setInterval(async () => {
      const level = await invoke("listen_level").catch(() => 0);
      bars.forEach((b, i) => { b.style.height = `${6 + Math.round(level * 34 * (0.5 + 0.5 * Math.abs(Math.sin(Date.now() / 120 + i))))}px`; });
    }, 120);
  }
  syncAttentive();
  if (greet) say("I'm listening.");
  refreshAgentControls();
  document.getElementById("agent-input").focus();
}

function closeAgent() {
  agent.open = false;
  agentEl.hidden = true;
  clearInterval(agent.levelTimer);
  speechSynthesis?.cancel();
  if (agent.mode === "confirm") agent.mode = "idle";
  syncAttentive();
  if (agent.forcedWake) {
    agent.forcedWake = false;
    syncWake().catch(() => {});
  }
}

/// What can be said right now: meeting controls while recording, otherwise starting things.
function agentCommands() {
  return recording
    ? ["Bookmark that", "Action item", "Stop recording", "What can I say?"]
    : ["Start meeting", "Take a note", "Add to last note", "Read back my last note", "What can I say?"];
}

function refreshAgentControls() {
  if (recording) {
    setAgentState(`Recording · ${clock(nowInRecording())}`);
    agentButtons(["Bookmark", () => run("bookmark that")], ["Action item", () => run("action item")], ["Stop recording", () => run("stop recording"), true]);
  } else if (agent.mode === "confirm") {
    const pending = COMMANDS.find((c) => c.id === agent.pending);
    setAgentState("Waiting for your answer");
    agentButtons(["No", () => run("no")], [pending ? `Yes: “${pending.say}”` : "Yes", () => run("yes"), true]);
  } else {
    setAgentState("Listening");
    agentButtons(["Close", () => closeAgent()]);
  }
  const links = agentCommands().flatMap((c, i) => [i ? " · " : "", el("a", { href: "#", "data-say": c }, `“${c}”`)]);
  document.getElementById("agent-say").replaceChildren("Say or click: ", ...links);
}

document.getElementById("agent-close").addEventListener("click", () => closeAgent());
document.getElementById("agent-type").addEventListener("submit", (e) => {
  e.preventDefault();
  const input = document.getElementById("agent-input");
  const text = input.value.trim();
  input.value = "";
  if (text) run(text);
});
agentEl.addEventListener("keydown", (e) => { if (e.key === "Escape") closeAgent(); });

listen("heard", (e) => onHeard(String(e.payload)));
listen("keyphrase", (e) => onKeyphrase(e.payload));

/// Commands trained as key phrases, which act as soon as they are heard, without speech to text.
const KEYPHRASE_COMMANDS = { bookmark: "bookmark", action: "action", stop: "stop-recording" };

/// When "Hey Superteem" was last heard during a dictation, so the words after it are a command.
let wokeAt = 0;
const justWoke = () => Date.now() - wokeAt < 6000;
/// "Stop recording", "stop the note", "stop dictating" and the like.
const STOP_WORDS = /^\W*(stop|end|finish|done|close)\b.*\b(record|note|dictat|writing)/i;

/// A trained phrase, heard by the keyword spotter about a tenth of a second after it was said.
function onKeyphrase(h) {
  if (speaking || enrolment || onboardEnrolling || phraseTraining) return;
  // The Settings page shows what was heard instead of acting on it.
  if (keyphrasePage) return keyphrasePage.onHeard(h);
  if (h.rejected) return;
  if (view.kind === "onboarding" && view.step === 2) {
    if (h.phrase === "wake") onboardTick();
    return;
  }
  // A dictation takes every word as text, apart from "Hey Superteem, stop recording".
  if (dictation) {
    if (h.phrase === "wake") {
      wokeAt = Date.now();
      chime();
    } else if (h.phrase === "stop") {
      stopNoteByVoice();
    }
    return;
  }
  if (h.phrase === "wake") {
    chime();
    // No greeting: speaking would talk over the command that usually follows straight away.
    if (!agent.open) openAgent(false);
    heardLine("Hey Superteem");
    setAgentState("Listening for a command…");
    // Said by mistake, or nothing followed: the panel steps back so speech isn't transcribed for nothing.
    clearTimeout(agent.quietTimer);
    agent.quietTimer = setTimeout(() => { if (agent.open && agent.mode === "idle") closeAgent(); }, 10_000);
    return;
  }
  const command = KEYPHRASE_COMMANDS[h.phrase];
  if (command) {
    if (!agent.open) openAgent(false);
    heardLine(h.text);
    doCommand(command);
  }
}

/// Ends a dictation when told to by voice.
function stopNoteByVoice() {
  wokeAt = 0;
  if (dictation) stopDictation(true);
}

function onHeard(text) {
  // Words read aloud for a voice print or key phrase aren't commands, even if they arrive late.
  if (speaking || enrolment || onboardEnrolling || phraseTraining) return;
  if (keyphrasePage) return keyphrasePage.onWords(text);
  const wake = WAKE.exec(text);
  if (view.kind === "onboarding" && view.step === 2) {
    if (wake) onboardTick();
    return;
  }
  // A dictation only listens for "Hey Superteem, stop recording"; its words are written at the tick.
  if (dictation) {
    const words = wake ? text.slice(wake.index + wake[0].length) : text;
    if ((wake || justWoke()) && STOP_WORDS.test(words)) stopNoteByVoice();
    return;
  }
  if (!agent.open && !wake) return;
  const rest = wake ? text.slice(wake.index + wake[0].length).trim() : text;
  if (!agent.open) openAgent(!rest);
  if (rest) {
    heardLine(text);
    run(rest, true);
  } else if (agent.open && wake) {
    heardLine(text);
    if (agent.mode !== "idle" || recording) refreshAgentControls();
  }
}

/// Answers one command, spoken or typed.
async function run(raw, alreadyShown = false) {
  clearTimeout(agent.quietTimer);
  if (!agent.open) await openAgent(false);
  if (!alreadyShown) heardLine(raw);
  const t = raw.toLowerCase().replace(/[^\w\s']/g, " ").replace(/\s+/g, " ").trim();
  const has = (re) => re.test(t);

  if (agent.mode === "confirm") {
    const pending = agent.pending;
    agent.mode = "idle";
    agent.pending = null;
    if (has(/^(yes|yeah|yep|yup|sure|please|do it|go ahead|correct|right|ok|okay)\b/)) return doCommand(pending);
    if (has(/^(no|nope|not|cancel|never ?mind|wrong)\b/)) {
      say("Okay. What would you like to do?");
      return refreshAgentControls();
    }
    // Anything else is taken as a new request.
  }

  const [best, ...others] = interpret(raw);
  if (best.score >= 1) return doCommand(best.id);
  const close = [best, ...others].filter((c) => c.score >= 0.5).slice(0, 3);
  if (close.length === 1) {
    agent.mode = "confirm";
    agent.pending = best.id;
    say(`Did you mean “${best.say}”?`);
    return refreshAgentControls();
  }
  const offer = close.length ? close : (recording ? SUGGESTED_RECORDING : SUGGESTED).map((id) => COMMANDS.find((c) => c.id === id));
  say(close.length ? "Did you mean one of these?" : "I'm not sure how to help with that yet. Here are some things I can do.");
  setAgentState("Pick one, or say it another way");
  agentButtons(...offer.map((c, n) => [c.say, () => { heardLine(c.say); doCommand(c.id); }, n === 0]));
}

/// Carries out one command, by its id in COMMANDS.
async function doCommand(id) {
  if (agent.mode === "confirm") agent.mode = "idle";
  if (id === "cancel") {
    agent.mode = "idle";
    say("Okay.");
    setTimeout(closeAgent, 900);
    return;
  }
  if (id === "stop-recording") {
    if (!recording) {
      say("Nothing is recording.");
    } else {
      say("Stopping. I'll write up the transcript now.");
      agent.mode = "idle";
      setTimeout(closeAgent, 1500);
      await stopMeeting();
    }
    return refreshAgentControls();
  }
  // "Start meeting", "record a meeting", "start recording": a new meeting starts recording straight away.
  if (id === "record") {
    if (recording) {
      say("Already recording. Say “stop recording” when you're done.");
    } else {
      try {
        const { ticked } = await rememberedInputs();
        await startMeeting(ticked, prefs.pins);
        say("Recording. Say “Hey Superteem, stop recording” when you're done.", true);
        setTimeout(() => { if (recording) closeAgent(); }, 4000);
      } catch (e) {
        say(`I couldn't start recording: ${e}`);
      }
    }
    return refreshAgentControls();
  }
  if (id === "bookmark") {
    say(addMarker("") ? `Bookmarked at ${clock(nowInRecording())}.` : "Bookmarks are for meetings. Nothing is recording.");
    if (recording) stepBackSoon();
    return refreshAgentControls();
  }
  if (id === "action") {
    say(addMarker("", "action") ? `Action item marked at ${clock(nowInRecording())}. It's written up with the transcript.` : "Action items are marked in meetings. Nothing is recording.");
    if (recording) stepBackSoon();
    return refreshAgentControls();
  }
  if (id === "add-note") {
    const [latest] = await invoke("list_notes", { query: "" });
    return latest ? noteByVoice(latest.id, `Adding to ${latest.title}.`) : noteByVoice(null, "There are no notes yet. Starting a new one.");
  }
  if (id === "note") {
    return noteByVoice(null, "New note.");
  }
  if (id === "stop-note") {
    say("No note is being dictated. Say “take a note” to start one.");
    return refreshAgentControls();
  }
  if (id === "read-note") {
    const [latest] = await invoke("list_notes", { query: "" });
    if (!latest) {
      say("There are no notes yet.");
    } else {
      const full = await invoke("get_note", { id: latest.id });
      say(`${latest.title}. ${full.body || "It's empty."}`);
    }
    return refreshAgentControls();
  }
  if (id === "help") {
    say(recording
      ? "You can say: bookmark that, action item, stop recording, or cancel."
      : "You can say: start meeting, take a note or start dictation, add to last note, read back my last note, open settings, or cancel.");
    return refreshAgentControls();
  }
  if (id === "settings") {
    say("Opening settings.");
    closeAgent();
    return showSettings();
  }
  if (id === "home") {
    say("Here are your notes and meetings.");
    closeAgent();
    return showHome();
  }
}

/// A note asked for by voice opens with dictation running, as the mic button does, and the tick writes it
/// down. Once phrases are trained, "Hey Superteem, stop note" writes it down too.
async function noteByVoice(id, intro) {
  const dictate = platform !== "android";
  const done = phrasesTrained ? "press the tick or say “Hey Superteem, stop note”" : "press the tick";
  say(dictate ? `${intro} Go ahead, and ${done} when you're done.` : intro);
  closeAgent();
  await openNote(id, { dictate });
}

// ---------- Start ----------

(async () => {
  try {
    platform = await invoke("platform");
  } catch {}
  document.body.dataset.platform = platform;
  try {
    prefs = await invoke("get_prefs");
    // The theme saved on this device wins over the browser storage, which the webview may clear.
    if (prefs.theme) applyTheme(prefs.theme);
  } catch {}
  if (prefs.wake && platform !== "android") {
    await invoke("set_wake", { on: true }).catch(() => { prefs.wake = false; });
  }
  await refreshPhrasesTrained();
  showListening();
  const status = await invoke("recording_status").catch(() => ({ recording: false }));
  if (status.recording && !enrolment) {
    view = { kind: "recorder" };
    showMeeting();
    // The recording is rebuilt from its status once the page draws; listen through it from then.
    setTimeout(() => syncWake().catch(() => {}), 1000);
  } else if (!prefs.onboarded) {
    showOnboarding(0);
  } else {
    showHome();
    // On a phone the list is home.
    if (matchMedia("(max-width: 720px)").matches) document.body.classList.add("show-list");
  }
})();
