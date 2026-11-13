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
/// Refreshes the list of apps playing sound on the meeting page.
let sourceTimer = null;
let stopping = false;
let player = null;
let platform = "windows";
/// Windows and the Mac record in the app, listen for "Hey Superteem" and open folders; Android and the
/// iPhone are phones, which do neither.
const isDesktop = () => platform === "windows" || platform === "macos";
const isPhone = () => platform === "android" || platform === "ios";
/// Where deleted meetings go, on the computers that have one.
const binName = () => platform === "macos" ? "Trash" : "Recycle Bin";
let progress = null;
// Meetings being processed one at a time, while another can be recorded: { current, waiting }.
let queue = { current: null, waiting: [] };
const queued = (id) => queue.current === id || queue.waiting.includes(id);
let prefs = { wake: false, quiet: false, onboarded: true, terms_accepted: 1, inputs: [], pins: false, mics: {}, scripts: [] };

const icon = {
  pen: '<path d="M4 20h4L19 9l-4-4L4 16z"></path>',
  book: '<path d="M5 4.5A1.5 1.5 0 016.5 3H19v15H6.5A1.5 1.5 0 005 19.5z"></path><path d="M5 19.5A1.5 1.5 0 006.5 21H19v-3"></path>',
  up: '<path d="M6 15l6-6 6 6"></path>',
  down: '<path d="M6 9l6 6 6-6"></path>',
  wave: '<path d="M4 9v6M8 6v12M12 10v4M16 7v10M20 10v4"></path>',
  phone: '<path d="M5 4h4l2 5-2.5 1.5a11 11 0 005 5L15 13l5 2v4a2 2 0 01-2 2A16 16 0 013 6a2 2 0 012-2z"></path>',
  calendar: '<rect x="3.5" y="5" width="17" height="15" rx="2"></rect><path d="M3.5 10h17M8 3v4M16 3v4"></path>',
  mic: '<rect x="9" y="3" width="6" height="12" rx="3"></rect><path d="M5 11a7 7 0 0014 0M12 18v3"></path>',
  bookmark: '<path d="M6 3h12v18l-6-4-6 4z"></path>',
  // A clipboard with a tick: something to be done, written down.
  action: '<path d="M9 4H6.5A1.5 1.5 0 005 5.5v14A1.5 1.5 0 006.5 21h11a1.5 1.5 0 001.5-1.5v-14A1.5 1.5 0 0017.5 4H15"></path><rect x="9" y="2.5" width="6" height="3.5" rx="1"></rect><path d="M8.5 13.5l2.5 2.5 4.5-5"></path>',
  save: '<path d="M5 3h11.5L20 6.5V19a2 2 0 01-2 2H6a2 2 0 01-2-2V5a2 2 0 011-2z"></path><path d="M8 3v5h7V3"></path><rect x="7.5" y="13" width="9" height="8"></rect>',
  export: '<path d="M12 15V3M7.5 7.5L12 3l4.5 4.5"></path><path d="M5 12v7a2 2 0 002 2h10a2 2 0 002-2v-7"></path>',
  import: '<path d="M12 3v12M7.5 10.5L12 15l4.5-4.5"></path><path d="M5 12v7a2 2 0 002 2h10a2 2 0 002-2v-7"></path>',
  keyboard: '<rect x="2.5" y="6" width="19" height="12" rx="2"></rect><path d="M6 10h.01M10 10h.01M14 10h.01M18 10h.01M6 14h.01M18 14h.01M9 14h6"></path>',
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
  chevron: '<path d="M9 6l6 6-6 6"></path>',
  down: '<path d="M6 9l6 6 6-6"></path>',
  search: '<circle cx="11" cy="11" r="7"></circle><path d="M20 20l-3.8-3.8"></path>',
  more: '<circle cx="5" cy="12" r="1.4" fill="currentColor"></circle><circle cx="12" cy="12" r="1.4" fill="currentColor"></circle><circle cx="19" cy="12" r="1.4" fill="currentColor"></circle>',
  folder: '<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"></path>',
  // Lucide's refresh-cw.
  refresh: '<path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"></path><path d="M21 3v5h-5"></path><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"></path><path d="M8 16H3v5"></path>',
  // Two overlapping pages, as a chat app's copy button.
  copy: '<rect x="9" y="9" width="13" height="13" rx="2"></rect><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path>',
};

/// Puts `text` on the clipboard. `html`, when given, goes with it so a rich editor pastes the formatting
/// and a plain field pastes the text. Returns whether it landed.
async function writeClipboard(text, html) {
  const plain = String(text ?? "");
  try {
    if (html && typeof ClipboardItem !== "undefined") {
      await navigator.clipboard.write([new ClipboardItem({
        "text/plain": new Blob([plain], { type: "text/plain" }),
        "text/html": new Blob([String(html)], { type: "text/html" }),
      })]);
      return true;
    }
    await navigator.clipboard.writeText(plain);
    return true;
  } catch {
    try {
      const area = document.createElement("textarea");
      area.value = plain;
      area.setAttribute("readonly", "");
      area.style.position = "fixed";
      area.style.left = "-9999px";
      document.body.append(area);
      area.select();
      const ok = document.execCommand("copy");
      area.remove();
      return ok;
    } catch {
      return false;
    }
  }
}

/// An icon button that copies what `text()` returns — a string, or { text, html } — and shows a tick
/// for a moment, the way a chat app copies one reply.
function copyIconButton(text, label = "Copy") {
  const button = el("button", { class: "icon-only copy-btn", type: "button", title: label, "aria-label": label }, svg("copy", 16));
  let timer = null;
  // The cursor stays where it was, so copying a note doesn't drop the caret out of it.
  button.addEventListener("mousedown", (e) => e.preventDefault());
  button.addEventListener("click", async () => {
    const value = typeof text === "function" ? text() : text;
    const plain = value && typeof value === "object" ? value.text : value;
    const html = value && typeof value === "object" ? value.html : null;
    const ok = await writeClipboard(plain, html);
    button.classList.toggle("copied", ok);
    const said = ok ? "Copied" : "Couldn't copy";
    button.title = said;
    button.setAttribute("aria-label", said);
    button.replaceChildren(svg(ok ? "check" : "close", 16));
    clearTimeout(timer);
    timer = setTimeout(() => {
      if (!button.isConnected) return;
      button.classList.remove("copied");
      button.title = label;
      button.setAttribute("aria-label", label);
      button.replaceChildren(svg("copy", 16));
    }, 1600);
  });
  return button;
}

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

/// How long ago, as "3h ago", for recent things; the date for older ones.
function agoText(unix) {
  if (!unix) return "";
  const s = Date.now() / 1000 - unix;
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  if (s < 7 * 86400) return `${Math.floor(s / 86400)}d ago`;
  return new Date(unix * 1000).toLocaleDateString(undefined, { dateStyle: "medium" });
}

/// A note's last save, as "Just now" or "4h ago", kept current while it shows.
let savedTimer = null;
function showSaved(status) {
  const draw = () => {
    const at = note?.savedAt;
    status.textContent = !at ? "New note" : Date.now() / 1000 - at < 60 ? "Just now" : agoText(at);
    status.title = at ? `Saved ${dateText(at)}` : "";
  };
  draw();
  clearInterval(savedTimer);
  savedTimer = setInterval(() => {
    if (!status.isConnected || !note) { clearInterval(savedTimer); savedTimer = null; return; }
    if (!/^(Editing|Not saved)/.test(status.textContent)) draw();
  }, 30000);
}

/// An Export button with a menu of `choices` ([format, label, detail]). `run(format)` writes the file and
/// returns its path, which is then offered to show in File Explorer.
function exportMenu(choices, run) {
  const wrap = el("span", { class: "export-menu" });
  const button = el("button", { class: "link export-button", type: "button", "aria-haspopup": "menu", "aria-expanded": "false", title: "Export" },
    svg("export", 16), el("span", { class: "wide-only" }, "Export"));
  const menu = el("div", { class: "menu", role: "menu", hidden: true });
  const status = el("div", { class: "export-status", role: "status", "aria-live": "polite", hidden: true });
  const close = () => { menu.hidden = true; button.setAttribute("aria-expanded", "false"); document.removeEventListener("click", outside, true); };
  const outside = (e) => { if (!wrap.contains(e.target)) close(); };
  // Placed against the window, so it stays on screen wherever the button is: on a phone the button is
  // not at the edge, and a menu hung from it would run off the left.
  const place = () => {
    const r = button.getBoundingClientRect();
    const width = Math.min(300, window.innerWidth - 16);
    Object.assign(menu.style, {
      position: "fixed", right: "auto", width: `${width}px`,
      left: `${Math.max(8, Math.min(r.right - width, window.innerWidth - width - 8))}px`,
      top: `${r.bottom + 6}px`, maxHeight: `${Math.max(160, window.innerHeight - r.bottom - 14)}px`, overflowY: "auto",
    });
  };
  button.addEventListener("click", () => {
    if (!menu.hidden) return close();
    menu.hidden = false;
    place();
    button.setAttribute("aria-expanded", "true");
    document.addEventListener("click", outside, true);
    menu.querySelector("button")?.focus();
  });
  window.addEventListener("resize", () => { if (!menu.hidden) place(); });
  menu.addEventListener("keydown", (e) => { if (e.key === "Escape") { close(); button.focus(); } });
  // On a computer, where exports go, and a way to choose another folder.
  if (isDesktop() && window.__TAURI__.dialog) {
    const where = el("small", {}, "");
    const showWhere = (path) => { where.textContent = `Saves to ${path.split(/[\\/]/).filter(Boolean).pop() ?? path}`; where.title = path; };
    invoke("export_dir").then(showWhere).catch(() => {});
    const change = el("button", { type: "button", role: "menuitem", class: "export-folder" }, el("strong", {}, "Choose the folder…"), where);
    change.addEventListener("click", async () => {
      close();
      const current = await invoke("export_dir").catch(() => undefined);
      const picked = await window.__TAURI__.dialog.open({ directory: true, defaultPath: current, title: "Export to this folder" }).catch(() => null);
      if (typeof picked === "string") {
        try { showWhere(await invoke("set_export_dir", { path: picked })); } catch (e) { showError(status, e); status.hidden = false; }
      }
    });
    menu.append(change);
  }
  for (const [format, label, detail] of choices) {
    const item = el("button", { type: "button", role: "menuitem" }, el("strong", {}, label), detail ? el("small", {}, detail) : null);
    item.addEventListener("click", async () => {
      close();
      status.hidden = false;
      status.replaceChildren(el("span", { class: "spinner" }), "Exporting…");
      try {
        const path = await run(format);
        const name = path.split(/[\\/]/).pop();
        status.replaceChildren(svg("check", 14), ` Exported ${name}`,
          isDesktop() ? el("button", { class: "link inline", type: "button", onclick: () => invoke("reveal_export", { path }).catch(() => {}) }, platform === "macos" ? "Show in Finder" : "Show in folder") : ` to ${path}`,
          el("button", { class: "icon-only", type: "button", "aria-label": "Dismiss", title: "Dismiss", onclick: () => { status.hidden = true; } }, svg("close", 14)));
      } catch (e) {
        status.replaceChildren(el("span", { class: "danger" }, String(e)),
          el("button", { class: "icon-only", type: "button", "aria-label": "Dismiss", title: "Dismiss", onclick: () => { status.hidden = true; } }, svg("close", 14)));
      }
    });
    menu.append(item);
  }
  wrap.append(button, menu);
  wrap.status = status;
  return wrap;
}

const TRANSCRIPT_EXPORTS = [
  ["txt", "Transcript as text", ".txt, with bookmarks and action items"],
  ["md", "Transcript as Markdown", ".md, for documents and wikis"],
  ["srt", "Subtitles", ".srt, timed lines for video"],
  ["vtt", "Web subtitles", ".vtt, timed lines with speakers"],
];

function timeText(unix) {
  return unix ? new Date(unix * 1000).toLocaleTimeString(undefined, { timeStyle: "short" }) : "";
}

function showError(target, error) {
  target.querySelector(".error")?.remove();
  target.append(el("div", { class: "error", role: "alert" }, String(error), micHelp(error)));
}

/// When a microphone problem may be the app's Android permissions or the system's microphone switch,
/// a button to the permissions page.
function micHelp(problem) {
  if (platform !== "android" || !/microphone|silence|access/i.test(String(problem))) return "";
  return el("div", { class: "actions" }, el("button", { class: "pill", onclick: () => showPermissions() }, "Check app permissions"));
}

/// On a phone the list and the page take turns filling the screen. Opening a page from the list leaves
/// a step in the history, so Android's Back gesture returns to the list rather than closing the app.
function showPage() {
  const fromList = document.body.classList.contains("show-list");
  document.body.classList.remove("show-list");
  if (fromList && !document.body.classList.contains("setup") && !history.state?.page) {
    history.pushState({ page: true }, "");
  }
}

function backToList() {
  leave();
  view = { kind: "home" };
  document.body.classList.add("show-list");
  refreshItems();
}

/// A meeting's top bar (and on a phone, a note's) stays in place, under "Back to Feed" on a phone, while the page scrolls
/// beneath it. Its height is kept in --pin-top, so the formatting bar can stay just under it.
let pinWatch = null;
function pinTopBar(bar) {
  unpinTopBar();
  document.body.classList.add("pin-top");
  const set = () => document.body.style.setProperty("--pin-top", `${pinnedHeight(bar)}px`);
  pinWatch = new ResizeObserver(set);
  pinWatch.observe(bar);
  pinWatch.observe(document.getElementById("back"));
  requestAnimationFrame(set);
}
function unpinTopBar() {
  pinWatch?.disconnect();
  pinWatch = null;
  document.body.classList.remove("pin-top");
}
/// How much of the page's top the pinned bars cover; nothing where the bar isn't pinned.
function pinnedHeight(bar) {
  if (!bar?.isConnected || getComputedStyle(bar).position !== "sticky") return 0;
  return document.getElementById("back").offsetHeight + bar.offsetHeight;
}

/// How many words a text has, counted as its language counts them where the browser can.
function wordCount(text) {
  let n = 0;
  try {
    for (const part of new Intl.Segmenter(undefined, { granularity: "word" }).segment(text)) if (part.isWordLike) n++;
  } catch {
    n = text.split(/\s+/).filter(Boolean).length;
  }
  return n;
}
function wordsText(n) {
  return `${n.toLocaleString()} word${n === 1 ? "" : "s"}`;
}

/// Leaves whatever page was open: stops playback and polling, and saves a note being edited.
function leave() {
  stopReading();
  keyphrasePage?.stop();
  keyphrasePage = null;
  document.body.classList.remove("setup");
  stopProbe();
  cancelOnboardReading();
  handOffDictation();
  stopPlayback();
  clearInterval(pollTimer);
  clearInterval(sourceTimer);
  flushNote();
  unpinTopBar();
  document.body.classList.remove("note-chat-open", "note-chat-page", "note-chat-float");
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
  const [notes, sessions, all, books] = await Promise.all([
    invoke("list_notes", { query }).catch(() => []),
    invoke("list_sessions", { query }).catch(() => []),
    query.trim() ? invoke("list_sessions", { query: "" }).catch(() => []) : null,
    invoke("list_books", { query }).catch(() => []),
  ]);
  nameMeetings(all ?? sessions);
  // Notes in a book are listed in their book; a search still finds them on their own.
  const inBook = new Set(query.trim() ? [] : books.flatMap((b) => b.notes));
  const items = [
    // Archived notes are left out of the list but still found by search.
    ...books.map((b) => ({ kind: "book", id: b.id, title: b.title, when: b.updated,
      sub: `Book · ${timeText(b.updated)} · ${b.notes.length} note${b.notes.length === 1 ? "" : "s"}` })),
    ...notes.filter((n) => (query.trim() || !n.archived) && !inBook.has(n.id)).map((n) => ({ kind: "note", id: n.id, title: n.title, when: n.updated, status: n.status,
      sub: [`Note · ${timeText(n.updated)}`, n.words ? `${n.words} words` : null, n.archived ? "Archived" : null].filter(Boolean).join(" · ") })),
    ...sessions.map((s) => ({ kind: "session", id: s.id, title: meetingTitle(s.id, s.title), when: s.created, status: s.status,
      sub: [`Meeting · ${timeText(s.created)}`, s.duration_s ? clock(s.duration_s) : null,
        queue.current === s.id ? "processing…" : queue.waiting.includes(s.id) ? "waiting to process" : null,
        s.needs_processing ? "not processed" : null, s.compacted ? "audio deleted" : null].filter(Boolean).join(" · ") })),
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
    const active = view.kind === it.kind && view.id === it.id;
    itemsNav.append(el("button", { class: "item" + (active ? " active" : "") + (it.status === "done" ? " done" : ""),
      onclick: () => (it.kind === "note" ? openNote(it.id) : it.kind === "book" ? openBook(it.id) : openSession(it.id)) },
      el("span", { class: "icon" + (it.kind === "session" ? " meeting" : "") }, svg({ note: "pen", book: "book", session: "wave" }[it.kind], 16)),
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
  // On a phone, the bottom bar (Settings) steps aside while the list is filtered.
  document.body.classList.toggle("searching", !!search.value.trim());
  clearTimeout(searchTimer);
  searchTimer = setTimeout(refreshItems, 200);
});
document.getElementById("open-settings").addEventListener("click", () => showSettings());
document.getElementById("open-shortcuts").addEventListener("click", () => showShortcuts());
// The wordmark is the Home button too.
const goHome = () => showHome();
document.querySelector(".brand").addEventListener("click", goHome);
// The list has one floating button, which opens Home to choose what to make.
document.getElementById("new-fab").addEventListener("click", goHome);
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
  // The step left when the page opened is used up, so Back from the list then closes the app.
  if (history.state?.page) history.back();
  else backToList();
});
window.addEventListener("popstate", () => {
  if (!matchMedia("(max-width: 720px)").matches || document.body.classList.contains("show-list")
    || document.body.classList.contains("setup")) return;
  // Back from a note's chat, when it covers the note, goes to the note.
  if (view.kind === "note" && document.body.classList.contains("note-chat-float") && document.body.classList.contains("note-chat-open")) {
    history.pushState({ page: true }, "");
    return noteChatNow?.close();
  }
  backToList();
});

// ---------- Home: four things to make ----------

function showHome() {
  leave();
  view = { kind: "home" };
  showPage();
  refreshItems();
  showNewHome();
}

/// Four things to make, two of them not built yet.
function showNewHome() {
  const kinds = [
    { name: "Note", sub: "Write or dictate a thought", icon: "pen", open: () => openNote(null) },
    { name: "Meeting", sub: "Record and transcribe", icon: "wave", open: () => showMeeting() },
    { name: "Call", sub: "Capture a phone call", icon: "phone" },
    { name: "Event", sub: "Notes for a talk or session", icon: "calendar" },
    { name: "Book", sub: "Gather notes into one place", icon: "book", open: () => newBook(), wide: true },
  ];
  const tiles = kinds.map((k) => el("button", {
    class: `new-tile${k.open ? "" : " soon"}${k.wide ? " wide" : ""}`, "aria-disabled": k.open ? null : "true",
    "aria-label": k.open ? `New ${k.name.toLowerCase()}` : `${k.name}, coming soon`,
    onclick: k.open ?? (() => micToast(`${k.name}s are coming soon`)),
  }, el("span", { class: "tile-head" }, svg(k.icon, 31), el("span", { class: "tile-name" }, k.name),
      k.open ? null : el("span", { class: "tile-badge" }, "Soon")),
    el("span", { class: "tile-sub" }, k.sub)));
  main.replaceChildren(el("div", { class: "home phone-home" },
    // The wordmark, as in the sidebar.
    el("div", { class: "brand home-brand", role: "img", "aria-label": "Voice by Superteem" },
      el("span", { class: "product" }, "Voice"), el("span", { class: "by" }, "by"),
      el("img", { class: "logo-light-mode", src: "logo-light-mode.png", alt: "" }),
      el("img", { class: "logo-dark-mode", src: "logo-dark-mode.png", alt: "" })),
    el("h1", {}, "Start something new"),
    el("div", { class: "new-grid" }, tiles),
    el("div", { class: "hint" }, "Pick one to start")));
  homeExtras();
}

/// Under Home's hint: news of an update, and a suggestion to optimise.
function homeExtras() {
  // For a day after an update, which version this is and what changed.
  invoke("just_updated").then((u) => {
    if (!u || view.kind !== "home") return;
    const hint = main.querySelector(".home > .hint");
    hint?.after(el("p", { class: "just-updated" }, svg("check", 14), `Updated to ${u.version}`, " · ",
      el("button", { class: "link inline", onclick: () => invoke("open_link", { url: u.notes }).catch(() => {}) }, "What's new")));
  }).catch(() => {});
  // Once a note and a meeting are made, a suggestion to optimise, when it would change something.
  optimiseNudge().then((card) => { if (card && view.kind === "home") main.querySelector(".home")?.append(card); }).catch(() => {});
}

// ---------- Books: notes gathered in order ----------

async function newBook() {
  try {
    openBook(await invoke("create_book", { title: "" }), { naming: true });
  } catch (e) {
    micToast(String(e));
  }
}

/// Scrolls that follow opening a note, so focusing the cursor doesn't count as the person scrolling.
let noteOpenQuiet = 0;

/// The note opens at the top. Focusing the end can pull the view down a moment later, so this is said again.
function openAtTop(node) {
  const go = () => {
    for (let box = node?.parentElement; box; box = box.parentElement) {
      const y = getComputedStyle(box).overflowY;
      if (y === "auto" || y === "scroll" || y === "overlay") box.scrollTop = 0;
    }
  };
  noteOpenQuiet = performance.now() + 500;
  go();
  requestAnimationFrame(go);
  setTimeout(go, 60);
  setTimeout(go, 240);
}

/// Down jumps to the end of a note while it is being scrolled. Up joins it once the top of the note
/// has left the screen. Both fade over a second after three seconds still.
function noteScrollButtons(topEl, { floor = null, cover = null } = {}) {
  const motion = () => (matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth");
  const up = el("button", { class: "jump-end jump-up", type: "button", title: "Go to the top", "aria-label": "Go to the top of the note", tabindex: "-1" }, svg("up", 20));
  const down = el("button", { class: "jump-end", type: "button", title: "Go to the end", "aria-label": "Go to the end of the note", tabindex: "-1" }, svg("down", 20));
  let timer = null;
  let box = null;
  const hide = () => { up.classList.remove("show"); down.classList.remove("show"); };
  const onScroll = (e) => {
    if (!down.isConnected) return document.removeEventListener("scroll", onScroll, true);
    if (performance.now() < noteOpenQuiet) return;
    const target = e.target === document ? document.scrollingElement : e.target;
    if (!(target instanceof Element) || !target.contains(topEl)) return;
    box = target;
    clearTimeout(timer);
    const frame = target.getBoundingClientRect();
    const seen = window.visualViewport;
    const floorTop = floor?.isConnected ? floor.getBoundingClientRect().top : Infinity;
    const bottom = Math.min(frame.bottom, floorTop, seen ? seen.offsetTop + seen.height : innerHeight);
    const visibleTop = Math.max(frame.top, cover?.isConnected ? cover.getBoundingClientRect().bottom : frame.top);
    const topGone = topEl.getBoundingClientRect().top < visibleTop - 8;
    const atEnd = target.scrollHeight - target.scrollTop - target.clientHeight < 120;
    down.classList.toggle("show", !atEnd);
    up.classList.toggle("show", topGone);
    const y = bottom - 54;
    const x = `${Math.min(frame.right, innerWidth) - 60}px`;
    down.style.top = `${y}px`;
    down.style.left = x;
    up.style.top = `${atEnd ? y : y - 50}px`;
    up.style.left = x;
    if (!atEnd || topGone) timer = setTimeout(hide, 3000);
    else hide();
  };
  for (const button of [up, down]) {
    button.addEventListener("mousedown", (e) => e.preventDefault());
    button.addEventListener("pointerenter", () => clearTimeout(timer));
    button.addEventListener("pointerleave", () => { clearTimeout(timer); timer = setTimeout(hide, 3000); });
  }
  down.addEventListener("click", () => box?.scrollTo({ top: box.scrollHeight, behavior: motion() }));
  up.addEventListener("click", () => box?.scrollTo({ top: 0, behavior: motion() }));
  document.addEventListener("scroll", onScroll, true);
  return [up, down];
}

/// A book kept for reading: a contents list, then one chapter at a time. Nothing in it can be changed.
async function openReadingBook(book, chapterId, find = null) {
  const mine = view;
  let contents = [];
  try {
    contents = await invoke("book_contents", { id: book.id });
  } catch (e) {
    main.replaceChildren(el("div", { class: "page" }, el("div", { class: "error" }, String(e))));
    return;
  }
  if (view !== mine) return;
  const index = chapterId ? contents.findIndex((c) => c.id === chapterId) : -1;
  if (index >= 0) {
    let note;
    try {
      note = await invoke("get_note", { id: chapterId });
    } catch (e) {
      main.replaceChildren(el("div", { class: "page" }, el("div", { class: "error" }, String(e))));
      return;
    }
    if (view !== mine) return;
    const chapter = contents[index];
    const kicker = [chapter.part, chapter.label].filter(Boolean).join(" · ");
    const body = el("div", { class: "book-chapter-body" });
    const fileBase = platform === "windows" || platform === "android" ? "http://stvnote.localhost/" : "stvnote://localhost/";
    if (note.format === "md") {
      try {
        body.innerHTML = NoteEditor.toHTML(note.body, (file) => `${fileBase}${note.id}/${encodeURIComponent(file)}`);
      } catch {
        body.textContent = note.body;
      }
    } else {
      body.append(...note.body.split(/\n{2,}/).filter((p) => p.trim()).map((p) => el("p", {}, p)));
    }
    const turn = () => el("div", { class: "book-turn" },
      el("button", { class: "pill", disabled: index === 0, onclick: () => openBook(book.id, { chapter: contents[index - 1].id }) }, "Previous"),
      el("span", { class: "book-kicker" }, `${index + 1} of ${contents.length}`),
      el("button", { class: "pill", disabled: index === contents.length - 1, onclick: () => openBook(book.id, { chapter: contents[index + 1].id }) }, "Next"));
    const heading = el("h1", {}, note.title || chapter.title);
    const bodyHolder = el("div", { class: "book-chapter-holder" }, body);
    const lang = () => itemLanguages(note)[0];
    let marks = null;
    // What is selected, or from where the text was last clicked; otherwise the whole chapter.
    const speak = readAloudButton(() => marks?.readingText() ?? `${HEADING}${heading.textContent}\n${body.innerText}`,
      "Read the chapter aloud (from where you click, or what you select)", { lang });
    const mark = el("button", { class: "icon-only", type: "button", title: "Bookmark what's selected, or where you clicked", "aria-label": "Bookmark" }, svg("bookmark", 16));
    mark.addEventListener("mousedown", (e) => e.preventDefault());
    mark.addEventListener("click", () => marks?.create());
    const page = el("div", { class: "page book book-read" },
      el("div", { class: "book-contents-bar" },
        el("button", { class: "link", onclick: () => openBook(book.id) }, "Contents"),
        el("span", { class: "top-actions" }, mark, speak)),
      kicker ? el("p", { class: "book-kicker" }, kicker) : "",
      heading,
      turn(), bodyHolder, turn(),
      ...noteScrollButtons(heading));
    main.replaceChildren(page);
    const surface = domSurface(body);
    marks = attachBookmarks({ surface, holder: bodyHolder, noteId: () => note.id, onRead: (text) => speak.readText(text, lang()) });
    openAtTop(page);
    revealSnippet(surface, find);
    return;
  }
  const rows = [];
  let part = "";
  contents.forEach((c, i) => {
    if (c.part && c.part !== part) {
      part = c.part;
      rows.push(el("div", { class: "group-label" }, c.part));
    }
    rows.push(el("div", { class: "row" },
      el("span", { class: "book-num" + (c.label ? " chapter" : "") }, c.label || String(i + 1)),
      el("button", { class: "label link-label", onclick: () => openBook(book.id, { chapter: c.id }) }, c.title,
        el("small", {}, wordsText(c.words)))));
  });
  const list = el("div", { class: "group book-notes" }, rows.length ? rows : el("div", { class: "row" }, el("span", { class: "label" }, "This book has no chapters.")));
  const search = bookSearch(book.id, list, (hit, snippet) => openBook(book.id, { chapter: hit.id, find: snippet }));
  const page = el("div", { class: "page book book-read" },
    el("div", { class: "book-head" }, el("span", { class: "book-icon" }, svg("book", 22)),
      el("h1", { class: "book-locked-title" }, book.title || "Untitled book"),
      el("span", { class: "reader-pill", title: "For reading: the chapters stay as they are" }, "Reader")),
    search.bar, search.results, list);
  main.replaceChildren(page);
  openAtTop(page);
}

/// What was last searched for in each book, so going back to its contents shows the same results.
const bookSearchQuery = new Map();

/// A search through every note in a book, under its title. While it holds words, what it finds takes the
/// place of `list`; `onPick(hit, snippet)` opens a chapter at a place found.
function bookSearch(bookId, list, onPick) {
  const input = el("input", { type: "search", placeholder: "Search this book", "aria-label": "Search this book", autocomplete: "off", enterkeyhint: "search" });
  const count = el("span", { class: "hint", "aria-live": "polite" });
  const clear = el("button", { class: "icon-only", type: "button", title: "Clear the search", "aria-label": "Clear the search", hidden: true }, svg("close", 16));
  const bar = el("div", { class: "note-search book-search", role: "search" }, svg("search", 16), input, count, clear);
  const results = el("div", { class: "book-results", hidden: true });
  let timer = null;
  let asked = 0;
  const plural = (n, one, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;
  async function run() {
    const q = input.value.trim();
    bookSearchQuery.set(bookId, input.value);
    clear.hidden = !input.value;
    const mine = ++asked;
    if (q.length < 2) {
      results.hidden = true;
      list.hidden = false;
      count.textContent = q ? "Keep typing" : "";
      return;
    }
    const hits = await invoke("search_book", { id: bookId, query: q }).catch(() => []);
    if (mine !== asked) return;
    const total = hits.reduce((n, h) => n + h.count, 0);
    count.textContent = total ? `${total} in ${plural(hits.length, "chapter")}` : "Not found";
    results.replaceChildren(...(hits.length ? hits.map((h) => el("div", { class: "group book-hit" },
      el("button", { class: "book-hit-head", type: "button", onclick: () => onPick(h, h.snippets[0]) },
        h.label ? el("span", { class: "book-num chapter" }, h.label) : null,
        el("span", { class: "label" }, h.title, el("small", {}, [h.part, `${plural(h.count, "match", "matches")}`].filter(Boolean).join(" · ")))),
      ...h.snippets.map((s) => el("button", { class: "book-snippet", type: "button", onclick: () => onPick(h, s) }, s.before, el("mark", {}, s.hit), s.after)),
      h.count > h.snippets.length ? el("div", { class: "book-hit-more hint" }, `and ${plural(h.count - h.snippets.length, "more place")}`) : null))
      : [el("p", { class: "hint" }, `No chapter has “${q}” in it.`)]));
    results.hidden = false;
    list.hidden = true;
  }
  input.addEventListener("input", () => { clearTimeout(timer); timer = setTimeout(run, 220); });
  input.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && input.value) { e.preventDefault(); input.value = ""; run(); }
    else if (e.key === "Enter") { e.preventDefault(); clearTimeout(timer); run(); }
  });
  clear.addEventListener("click", () => { input.value = ""; run(); input.focus(); });
  input.value = bookSearchQuery.get(bookId) ?? "";
  if (input.value) run();
  return { bar, results };
}

/// Shows a place found by a book's search in a chapter's text: highlighted, in the middle of the window.
/// Waits for the page to finish opening at its top first.
function revealSnippet(surface, snippet) {
  if (!snippet) return;
  setTimeout(() => {
    if (!surface.root.isConnected) return;
    const found = findBookmark(surface.text(), {
      quote: snippet.hit, prefix: snippet.before.replace(/^…/, ""), suffix: snippet.after.replace(/…$/, ""), at: 0,
    });
    surface.reveal(found.from, found.to);
  }, 320);
}

/// A book's page: its notes in order, to open, reorder, take out or add to; or all of them read as one.
/// A book that isn't editable opens as a reading copy instead.
async function openBook(id, { naming = false, reading = false, chapter = null, find = null } = {}) {
  leave();
  showPage();
  const mine = view = { kind: "book", id };
  refreshItems();
  let book;
  try {
    book = await invoke("get_book", { id });
  } catch (e) {
    main.replaceChildren(el("div", { class: "page" }, el("div", { class: "error" }, String(e))));
    return;
  }
  if (view !== mine) return;
  if (book.editable === false) {
    await openReadingBook(book, chapter, find);
    return;
  }
  const [chapters, allNotes] = await Promise.all([
    invoke("book_chapters", { id }).catch(() => []),
    reading ? [] : invoke("list_notes", { query: "" }).catch(() => []),
  ]);
  if (view !== mine) return;
  const reopen = (opts = {}) => openBook(id, { reading, ...opts });

  const title = el("input", { class: "note-title", placeholder: "Untitled book", value: book.title, "aria-label": "Book title" });
  let titleTimer = null;
  const saveTitle = () => { clearTimeout(titleTimer); invoke("rename_book", { id, title: title.value }).then(refreshItems).catch(() => {}); };
  title.addEventListener("input", () => { clearTimeout(titleTimer); titleTimer = setTimeout(saveTitle, 500); });
  title.addEventListener("change", saveTitle);

  const remove = el("button", { class: "icon-only danger", title: "Delete book", "aria-label": "Delete book" }, svg("trash", 18));
  remove.addEventListener("click", async () => {
    if (!remove.dataset.armed) {
      remove.dataset.armed = "1";
      remove.classList.add("armed");
      remove.replaceChildren(svg("trash", 16), "Click again to delete the book (its notes stay)");
      setTimeout(() => { delete remove.dataset.armed; remove.classList.remove("armed"); remove.replaceChildren(svg("trash", 18)); }, 4000);
      return;
    }
    await invoke("delete_book", { id }).catch(() => {});
    showHome();
  });
  const modes = el("div", { class: "book-mode", role: "group", "aria-label": "View" },
    el("button", { class: "pill" + (reading ? "" : " on"), "aria-pressed": String(!reading), onclick: () => reopen({ reading: false }) }, "Notes"),
    el("button", { class: "pill" + (reading ? " on" : ""), "aria-pressed": String(reading), onclick: () => reopen({ reading: true }) }, "Read"));
  const page = el("div", { class: "page book" },
    el("div", { class: "book-head" }, el("span", { class: "book-icon" }, svg("book", 22)), title, remove), modes);

  if (reading) {
    // All the notes one after another, each under its title, which opens it.
    const fileBase = platform === "windows" || platform === "android" ? "http://stvnote.localhost/" : "stvnote://localhost/";
    if (!chapters.length) page.append(el("p", { class: "hint" }, "This book has no notes yet."));
    // Read aloud reads the whole book; or what is selected, or from where it was last clicked to the end.
    const attached = [];
    let lastClicked = null;
    const chapterText = (a) => `${HEADING}${a.heading}\n${a.surface.text()}`;
    const speak = readAloudButton(() => {
      const from = attached.find((a) => a.bodyEl.contains(getSelection()?.anchorNode)) ?? lastClicked;
      const i = from ? attached.indexOf(from) : -1;
      const start = from?.marks.readingText();
      if (i < 0 || start == null) return attached.map(chapterText).join("\n\n");
      // A selection is read on its own; from a place clicked, reading goes on into the chapters after it.
      const sel = from.surface.selection();
      if (sel && sel.from !== sel.to) return start;
      return [start, ...attached.slice(i + 1).map(chapterText)].join("\n\n");
    }, "Read the book aloud (from where you click, or what you select)");
    if (chapters.length) modes.append(el("span", { class: "grow" }), speak);
    let part = "";
    for (const c of chapters) {
      if (c.part && c.part !== part) {
        part = c.part;
        page.append(el("h2", { class: "book-part" }, c.part));
      }
      const body = el("div", { class: "book-chapter-body" });
      if (c.format === "md") {
        try {
          body.innerHTML = NoteEditor.toHTML(c.body, (file) => `${fileBase}${c.id}/${encodeURIComponent(file)}`);
        } catch {
          body.textContent = c.body;
        }
      } else {
        body.append(...c.body.split(/\n{2,}/).filter((p) => p.trim()).map((p) => el("p", {}, p)));
      }
      const heading = c.label ? `${c.label}. ${c.title}` : c.title;
      const holder = el("div", { class: "book-chapter-holder" }, body);
      page.append(el("section", { class: "book-chapter" },
        el("h2", {}, el("button", { class: "link", onclick: () => openNote(c.id) }, heading)), holder));
      attached.push({ c, heading, bodyEl: body, holder });
    }
    const top = page.querySelector("h1, h2") || page;
    page.append(...noteScrollButtons(top));
    main.replaceChildren(page);
    for (const a of attached) {
      a.surface = domSurface(a.bodyEl);
      a.marks = attachBookmarks({ surface: a.surface, holder: a.holder, noteId: () => a.c.id, onRead: (text, { toEnd } = {}) => {
        const i = attached.indexOf(a);
        speak.readText(toEnd ? [text, ...attached.slice(i + 1).map(chapterText)].join("\n\n") : text);
      } });
      a.bodyEl.addEventListener("click", () => { lastClicked = a; });
    }
    openAtTop(page);
    return;
  }

  const order = chapters.map((c) => c.id);
  const move = async (i, by) => {
    const next = [...order];
    [next[i], next[i + by]] = [next[i + by], next[i]];
    await invoke("reorder_book", { id, order: next }).catch(() => {});
    reopen();
  };
  const rows = [];
  let part = "";
  chapters.forEach((c, i) => {
    if (c.part && c.part !== part) {
      part = c.part;
      rows.push(el("div", { class: "group-label" }, c.part));
    }
    const otherPart = (j) => c.part && chapters[j] && chapters[j].part !== c.part;
    rows.push(el("div", { class: "row" },
      el("span", { class: "book-num" + (c.label ? " chapter" : "") }, c.label || String(i + 1)),
      el("button", { class: "label link-label", onclick: () => openNote(c.id) }, c.title,
        el("small", {}, `${timeText(c.updated)} · ${c.words} word${c.words === 1 ? "" : "s"}`)),
      el("button", { class: "icon-only", title: "Move up", "aria-label": `Move ${c.title} up`, disabled: i === 0 || otherPart(i - 1), onclick: () => move(i, -1) }, svg("up", 16)),
      el("button", { class: "icon-only", title: "Move down", "aria-label": `Move ${c.title} down`, disabled: i === chapters.length - 1 || otherPart(i + 1), onclick: () => move(i, 1) }, svg("down", 16)),
      el("button", { class: "pill", onclick: async () => { await invoke("remove_note_from_book", { id, note: c.id }).catch(() => {}); reopen(); } }, "Take out")));
  });
  const list = el("div", { class: "group book-notes" }, rows.length ? rows : el("div", { class: "row" }, el("span", { class: "label" }, "No notes yet. Write a new one, or add notes you have.")));

  const newNote = el("button", { class: "pill", onclick: async () => {
    try {
      const n = await invoke("save_note", { id: null, title: "", body: "", keepAudio: false, rich: false, doc: null });
      await invoke("add_note_to_book", { id, note: n });
      openNote(n);
    } catch (e) { micToast(String(e)); }
  } }, svg("pen", 14), "New note in this book");
  const others = allNotes.filter((n) => !order.includes(n.id));
  const picker = el("select", { "aria-label": "Add a note you have", disabled: !others.length },
    el("option", { value: "" }, others.length ? "Add a note you have…" : "No other notes to add"),
    ...others.map((n) => el("option", { value: n.id }, n.title)));
  picker.addEventListener("change", async () => {
    if (!picker.value) return;
    await invoke("add_note_to_book", { id, note: picker.value }).catch((e) => micToast(String(e)));
    reopen();
  });
  const search = bookSearch(id, list, async (hit, snippet) => {
    await openNote(hit.id);
    if (note?.id === hit.id && note.ed) revealSnippet(note.ed.surface, snippet);
  });
  modes.before(search.bar);
  page.append(search.results, list, el("div", { class: "book-add" }, newNote, picker));
  main.replaceChildren(page);
  if (naming) title.focus();
}

// ---------- Notes: a text editor that listens ----------

let note = null; // { id, title, body, dirty }
let noteTimer = null;

/// `dictate` starts with the cursor in the body and dictation running; left out, only a new note does,
/// unless Settings says to wait for Dictate.
async function openNote(id, { dictate } = {}) {
  const reading = id && await invoke("book_for_note", { id }).catch(() => null);
  if (reading) {
    openBook(reading, { chapter: id });
    return;
  }
  leave();
  showPage();
  let data = { id: null, title: "", body: "" };
  if (id) {
    // A dictation left on this note may still be writing its words in: those come first, so nothing typed
    // meanwhile is saved over them.
    await dictationHandoff;
    if (await invoke("dictation_writing", { id }).catch(() => false)) {
      main.replaceChildren(el("div", { class: "page" }, el("p", { class: "hint" }, "Writing in your dictation…")));
      const mine = view = { kind: "note", id };
      await new Promise((resolve) => {
        const check = async () => ((await invoke("dictation_writing", { id }).catch(() => false)) ? setTimeout(check, 400) : resolve());
        check();
      });
      if (view !== mine) return;
    }
    try {
      data = await invoke("get_note", { id });
    } catch (e) {
      main.replaceChildren(el("div", { class: "page" }, el("div", { class: "error" }, String(e))));
      return;
    }
  }
  // `body` is Markdown once the note has been written in the formatting editor (`rich`); `doc` is the editor's own copy.
  note = { id: data.id, title: data.title, body: data.body, rich: data.format === "md", doc: data.doc ?? null, keepAudio: !!data.keep_audio, archived: !!data.archived, status: data.status ?? "", dirty: false };
  view = { kind: "note", id: data.id };
  refreshItems();
  await languageData();
  renderNote(data, dictate);
}

function renderNote(data, dictate) {
  const status = el("span", { class: "status", "aria-live": "polite" });
  note.savedAt = data.updated ?? null;
  showSaved(status);
  // The words show only in the phone's "more" menu, where the buttons are rows.
  const menuText = (text) => el("span", { class: "menu-text" }, text);
  const remove = el("button", { class: "icon-only danger", title: "Delete", "aria-label": "Delete note" }, svg("trash", 18), menuText("Delete"));
  remove.addEventListener("click", async () => {
    if (!note.id) return showHome();
    if (!remove.dataset.armed) {
      remove.dataset.armed = "1";
      remove.classList.add("armed");
      remove.replaceChildren(svg("trash", 16), "Click again to delete");
      setTimeout(() => {
        delete remove.dataset.armed;
        remove.classList.remove("armed");
        remove.replaceChildren(svg("trash", 18), menuText("Delete"));
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
    archive.replaceChildren(svg(note.archived ? "unarchive" : "archive", 18), menuText(label));
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
        note.id = await invoke("save_note", noteArgs(null));
        view.id = note.id;
        note.dirty = false;
        note.syncStack?.();
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
  note.languages = itemLanguages(data);
  const noteLanguage = () => note.languages[0];
  const langNotice = el("div", { class: "notice language-notice", hidden: true, "aria-live": "polite" });
  // The formatting editor: headings, lists, links, pictures and files, with places held for dictation.
  const mine = note;
  const noteFiles = `${platform === "windows" || platform === "android" ? "http://stvnote.localhost/" : "stvnote://localhost/"}`;
  const fileUrl = (file) => `${noteFiles}${mine.id ?? ""}/${encodeURIComponent(file)}`;
  const openLink = (href) => invoke("open_note_link", { id: mine.id ?? "", href }).catch((e) => micToast(String(e)));
  const editorHolder = el("div", { class: "note-editor" });
  const ed = NoteEditor.create({
    element: editorHolder,
    placeholder: isPhone()
      ? "Start typing, or press Dictate and talk. Your words appear at the cursor each time you pause, and it stops by itself when you finish."
      : "Start typing, or press Dictate (Ctrl + Space) and talk. Your words appear at the cursor each time you pause, and it stops by itself when you finish.",
    host: {
      onChange: () => changed(),
      onSearchJump: () => closeSearch(),
      notify: (text) => micToast(text),
      resolve: fileUrl,
      open: openLink,
      // A file is kept beside the note, so a new note is saved first to have somewhere to keep it.
      addFile: async (file) => {
        const id = await mine.save();
        return invoke("add_note_file", new Uint8Array(await file.arrayBuffer()), { headers: { "x-note": id, "x-file-name": encodeURIComponent(file.name || "file") } });
      },
      importImage: async (url) => invoke("import_note_image", { id: await mine.save(), url }),
    },
  });
  ed.setFrom(data.doc ? { doc: data.doc } : data.format === "md" ? { markdown: data.body } : { plain: data.body });
  const body = ed.dom;
  // Sits at the end of the note, as under a chat app's reply, with how many words it will copy: the note
  // as it reads, and as formatted text for a rich paste.
  const copyNote = copyIconButton(() => {
    const heading = title.value.trim();
    const plain = [heading, ed.text().trim()].filter(Boolean).join("\n\n");
    let html = "";
    try {
      const safe = heading.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
      html = `${heading ? `<h1>${safe}</h1>` : ""}${NoteEditor.toHTML(ed.markdown(), () => "")}`;
    } catch { /* The plain text still copies. */ }
    return { text: plain, html };
  }, "Copy note");
  const copyCount = el("span", { class: "hint" });
  const noteEnd = el("div", { class: "note-end", hidden: true }, copyNote, copyCount);
  let countTimer = null;
  const showCount = () => {
    const n = wordCount(`${title.value} ${ed.text()}`);
    noteEnd.hidden = !n;
    copyCount.textContent = wordsText(n);
  };
  const countSoon = () => { clearTimeout(countTimer); countTimer = setTimeout(showCount, 300); };
  note.ed = ed;
  const chip = el("div", { class: "dictation-chip", id: "dictation-chip", hidden: true, "aria-live": "polite" });
  const changed = () => {
    note.title = title.value;
    note.body = ed.markdown();
    note.doc = ed.json();
    note.rich = true;
    note.dirty = true;
    status.textContent = "Editing…";
    countSoon();
    clearTimeout(noteTimer);
    noteTimer = setTimeout(() => saveNote(status), 600);
  };
  title.addEventListener("input", changed);

  // Search: the note folds to its chapters (Heading 1) and sections (Heading 2), and shows only what holds
  // the word typed, under its headings. A click on a heading opens it; a click on text unfolds the note there.
  const searchInput = el("input", { type: "search", placeholder: "Search this note", "aria-label": "Search this note", autocomplete: "off", enterkeyhint: "search" });
  const searchCount = el("span", { class: "hint", "aria-live": "polite" });
  const searchBar = el("div", { class: "note-search", role: "search", hidden: true }, svg("search", 16), searchInput, searchCount,
    el("button", { class: "icon-only", type: "button", title: "Close search", "aria-label": "Close search", onclick: () => closeSearch() }, svg("close", 16)));
  const searchLink = el("button", { class: "link add-analysis", type: "button", title: "Search this note", "aria-label": "Search this note", "aria-expanded": "false",
    onclick: () => (ed.search.active() ? closeSearch() : openSearch()) },
    el("span", { class: "with-icon" }, svg("search", 16), el("span", { class: "wide-only" }, "Search")));
  const showFound = (found, at) => {
    searchCount.textContent = !searchInput.value.trim() ? (ed.search.structured() ? "Tap a chapter, or type" : "")
      : !found ? "Not found" : at ? `${at} of ${found}` : `${found} found`;
  };
  // Anything pressed outside the note and the search closes it.
  const outsideSearch = (e) => {
    if (!searchBar.isConnected) return document.removeEventListener("pointerdown", outsideSearch, true);
    if (!searchBar.contains(e.target) && !body.contains(e.target) && !searchLink.contains(e.target)) closeSearch();
  };
  function openSearch() {
    ed.search.open();
    searchBar.hidden = false;
    searchBar.style.top = `${pinnedHeight(topBar) + ed.toolbar.offsetHeight + 6}px`;
    searchLink.classList.add("on");
    searchLink.setAttribute("aria-expanded", "true");
    searchInput.value = "";
    found = 0;
    showFound(0);
    ed.scroller().scrollTop = 0;
    searchInput.focus();
    document.addEventListener("pointerdown", outsideSearch, true);
  }
  function closeSearch() {
    if (!ed.search.active()) return;
    document.removeEventListener("pointerdown", outsideSearch, true);
    searchBar.hidden = true;
    searchLink.classList.remove("on");
    searchLink.setAttribute("aria-expanded", "false");
    ed.search.close();
  }
  note.closeSearch = closeSearch;
  let found = 0;
  searchInput.addEventListener("input", () => { found = ed.search.query(searchInput.value); showFound(found); });
  searchInput.addEventListener("keydown", (e) => {
    if (e.key === "Escape") { e.preventDefault(); closeSearch(); }
    // Enter goes through what was found, one place at a time.
    else if (e.key === "Enter") { e.preventDefault(); if (found) showFound(found, ed.search.step(e.shiftKey)); }
  });

  const foot = el("div", { class: "toolbar note-foot" });
  // Down to the end, and up to the top once the title has scrolled away. They fade after the scrolling stops.
  const jumps = noteScrollButtons(title, { floor: foot, cover: ed.toolbar });

  // "Analysis templates", "Versions" and "Translate" sit under the title from the start: greyed out
  // until the note has something in it and is saved, then the real ones take their place.
  // On a phone it gives the note's version number instead, "V3", so the line fits.
  const versionNumber = el("span", { class: "narrow-only" });
  note.showVersion = async () => {
    const n = note.id ? await invoke("note_version_number", { id: note.id }).catch(() => 0) : 0;
    versionNumber.textContent = n ? `V${n}` : "";
    versionNumber.parentElement?.closest("button")?.setAttribute("aria-label", n ? `Versions: this is version ${n}` : "Versions");
  };
  const versionsLink = () => el("button", { class: "link add-analysis", type: "button", title: "Earlier versions of this note", onclick: () => showNoteVersions(status) },
    el("span", { class: "with-icon" }, svg("history", 16), el("span", { class: "wide-only" }, "Versions"), versionNumber));
  const stackSlot = el("div", { class: "analyses-slot" });
  let stackFor = null;
  note.syncStack = () => {
    if (stackFor && stackFor === note.id) return;
    if (note.id && note.body.trim()) {
      stackFor = note.id;
      stackSlot.replaceChildren(analysisStack("note", note.id, { speakers: ["Me"], lang: noteLanguage, go: (source) => { closeSearch(); ed.select(source.text); },
        lead: [searchLink], links: [versionsLink()], translations: translationStack("note", note.id, note.languages) }));
      note.showVersion();
    } else if (!stackSlot.firstChild) {
      const off = (name, text) => el("button", { class: "link add-analysis", type: "button", disabled: true, title: "Write something in the note first" },
        el("span", { class: "with-icon" }, svg(name, 16), text));
      stackSlot.replaceChildren(el("section", { class: "analyses", "aria-label": "Analyses and translations" },
        el("div", { class: "analysis-add" }, searchLink, off("sparkle", el("span", {}, "Analysis", el("span", { class: "wide-only" }, " templates"))), el("span", { class: "grow" }),
          off("history", el("span", { class: "wide-only" }, "Versions")), off("globe", "Translate"))));
    }
  };
  // Puts a text the app made in the editor: an assistant's rewrite, or an earlier version brought back.
  note.apply = (fresh) => {
    closeSearch();
    clearTimeout(noteTimer);
    title.value = fresh.title;
    ed.setFrom(fresh.doc ? { doc: fresh.doc } : fresh.format === "md" ? { markdown: fresh.body } : { plain: fresh.body });
    Object.assign(note, { title: fresh.title, body: fresh.body, rich: fresh.format === "md", doc: fresh.doc ?? null, dirty: false, savedAt: fresh.updated });
    showSaved(status);
    showCount();
    refreshItems();
    refreshAnalyses();
    note.showVersion?.();
  };
  note.editorText = () => ed.markdown();
  note.addText = (text) => ed.append(text);
  note.fileUrl = fileUrl;
  note.openLink = openLink;
  note.save = async () => {
    clearTimeout(noteTimer);
    await saveNote(status);
    if (!note.id) {
      note.id = await invoke("save_note", noteArgs(null));
      view.id = note.id;
      note.dirty = false;
      refreshItems();
    }
    note.syncStack();
    return note.id;
  };
  const chat = noteChat(status);
  title.addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); body.focus(); } });

  const talk = el("button", { class: "talk", id: "talk", onclick: () => { closeSearch(); startDictation(); } }, svg("mic", 18), "Dictate");
  // Pressing Dictate leaves the cursor where it was, so the words can go there.
  talk.addEventListener("mousedown", (e) => e.preventDefault());
  const live = el("div", { class: "dictating", id: "dictating", hidden: true, "aria-live": "polite" });

  const exporter = exportMenu([
    ["txt", "Text", ".txt"],
    ["md", "Markdown", ".md"],
    ["package", "With dictation audio", "The note and the audio of its dictations, while they are kept"],
  ], async (format) => {
    // What's on screen is saved first, so the export has it.
    clearTimeout(noteTimer);
    await saveNote(status);
    if (!note.id) throw new Error("Write something in the note first.");
    return invoke("export_note", { id: note.id, format });
  });
  // Reads what is selected, or from where the note was last clicked; otherwise the whole note, title first.
  let marks = null;
  const speakNote = readAloudButton(() => marks?.readingText() ?? `${HEADING}${title.value}\n${ed.text()}`,
    "Read the note aloud (from where you click, or what you select)", { lang: noteLanguage });
  const markButton = el("button", { class: "icon-only", type: "button", title: "Bookmark what's selected, or the cursor's place", "aria-label": "Bookmark" }, svg("bookmark", 16), menuText("Bookmark"));
  markButton.addEventListener("mousedown", (e) => e.preventDefault());
  markButton.addEventListener("click", () => marks?.create());
  // On a phone, Export, Archive and Delete sit behind one "more" button, so the top bar keeps to one line.
  const more = moreMenu([exporter, archive, remove]);
  foot.append(el("span", { class: "grow" }), talk, live);
  // On a computer the chat runs the full height of the page, and the top bar and the note sit beside it.
  const topBar = el("div", { class: "topbar" }, picker, languagePicker(note.languages, async (list) => {
      try {
        if (!note.id) {
          note.id = await invoke("save_note", noteArgs(null));
          view.id = note.id;
          note.dirty = false;
          note.syncStack?.();
        }
      } catch (e) {
        status.textContent = `Not saved: ${e}`;
        return false;
      }
      const result = await changeItemLanguages("note", note.id, list, langNotice);
      if (!result) return false;
      note.languages = result.effective;
      setLanguage([title, body], noteLanguage());
      return true;
    }), status,
      // The buttons stay together: beside the status, or on the line under it when they don't fit.
      el("span", { class: "top-actions" }, markButton, speakNote, more));
  main.replaceChildren(el("div", { class: "note-split" }, el("div", { class: "note-main" },
    topBar, exporter.status, langNotice,
    // The note scrolls under the top bar; the chat folds to a thin bar on the right.
    el("div", { class: "note-col" },
      ed.toolbar,
      el("div", { class: "note" }, title, stackSlot, searchBar, chip, editorHolder, noteEnd, ...jumps),
      foot)),
    chat?.panel));
  marks = attachBookmarks({ surface: ed.surface, holder: editorHolder, noteId: () => mine.id, ensureNote: () => mine.save(),
    onRead: (text) => speakNote.readText(text) });
  note.syncStack();
  chat?.sync();
  pinTopBar(topBar);
  showCount();
  setLanguage([title, body], noteLanguage());
  // A new note starts dictating straight away unless Settings says not to. Every note opens at its top,
  // with the cursor at its end for what is written or dictated next. On a phone the keyboard waits for a tap
  // unless dictation is starting.
  const dictating = dictate ?? (!data.id && !prefs.dictate_manually);
  // Quiet first, so focusing the end doesn't flash the jump buttons before the view is put back at the top.
  openAtTop(title);
  ed.cursorEnd(!isPhone() || dictating);
  if (dictating) startDictation();
}

/// What save_note is given: the note's Markdown, and the editor's document with it.
function noteArgs(id) {
  return { id, title: note.title, body: note.body, keepAudio: note.keepAudio, rich: note.rich, doc: note.rich ? note.doc : null };
}

async function saveNote(status) {
  if (!note || !note.dirty) return;
  if (!note.id && !note.title.trim() && !note.body.trim()) return;
  note.dirty = false;
  try {
    const id = await invoke("save_note", noteArgs(note.id));
    note.id = id;
    if (view.kind === "note") view.id = id;
    note.savedAt = Date.now() / 1000;
    if (status?.isConnected) showSaved(status);
    note.syncStack?.();
    note.showVersion?.();
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

/// Holds a dictation's place in the note: the marker its clips are written in before, in order.
function placeDictationSlot() {
  const places = note.ed.dictation;
  const id = places.place();
  return { id, classList: { add() {} }, replaceChildren() {}, remove: () => places.end(id, true), isConnected: true };
}

/// Takes the marker out again, along with the line made for it, when nothing was written.
function removeDictationSlot(slot) {
  if (slot) note?.ed?.dictation.end(slot.id, false);
}

let dictation = null; // { session, anchor, slots: Map(clip → slot), levelTimer, clockTimer, busy, wrote }

/// A clip was cut at a pause and is being written down while the next one records: a widget holds its
/// place, after the clips before it.
listen("dictation-clip", (e) => {
  if (dictation?.session !== e.payload.session) return;
  dictation.slots.set(e.payload.clip, clipSlot());
});
/// A clip's words take the place of its widget.
listen("dictation-words", (e) => {
  if (dictation?.session !== e.payload.session) return;
  const slot = dictation.slots.get(e.payload.clip);
  dictation.slots.delete(e.payload.clip);
  fillSlot(slot, e.payload.text);
  if (dictation.finished) tidyDictation();
});
/// A silence after an empty clip ended it: a chirp and a buzz say it stopped.
listen("dictation-ended", (e) => {
  if (dictation?.session !== e.payload.session || dictation.busy) return;
  dictation.busy = true;
  endDictationUi();
  if (e.payload.text?.trim()) fillSlot(clipSlot(), e.payload.text);
  stoppedSignal();
  finishDictation();
});

/// A "Writing it down…" widget for a clip, just before the marker where the dictation goes.
function clipSlot() {
  const places = note?.ed?.dictation;
  const id = places?.clip(dictation.anchor.id);
  return { id, remove: () => places?.remove(id) };
}

/// Replaces a clip's widget with its words, or takes it away when nothing was said.
function fillSlot(slot, words) {
  if (!slot || !note?.ed) return;
  if (note.ed.dictation.fill(slot.id, words) && dictation) dictation.wrote = true;
}

/// A short falling chirp, and a buzz where the device has one, when dictation stops by itself.
function stoppedSignal() {
  try {
    const audio = new (window.AudioContext || window.webkitAudioContext)();
    const tone = audio.createOscillator();
    const gain = audio.createGain();
    tone.type = "sine";
    tone.frequency.setValueAtTime(1320, audio.currentTime);
    tone.frequency.exponentialRampToValueAtTime(660, audio.currentTime + 0.18);
    gain.gain.setValueAtTime(0.0001, audio.currentTime);
    gain.gain.exponentialRampToValueAtTime(0.18, audio.currentTime + 0.02);
    gain.gain.exponentialRampToValueAtTime(0.0001, audio.currentTime + 0.25);
    tone.connect(gain).connect(audio.destination);
    tone.start();
    tone.stop(audio.currentTime + 0.27);
    setTimeout(() => audio.close().catch(() => {}), 500);
  } catch {}
  if (isPhone()) invoke("haptic").catch(() => {});
}

/// A button beside a text field that records the person speaking, for up to two minutes, and puts the
/// words in the field at its cursor. A second press stops it; stopping by itself chirps and buzzes.
const VOICE_LIMIT_S = 120;
let voiceRecording = null; // The voice button now recording, so only one records at a time.
function voiceButton(field, { onWords } = {}) {
  const label = el("span", { class: "rec-time" });
  const b = el("button", { class: "icon-only voice-btn", type: "button", title: "Speak instead of typing (up to two minutes)", "aria-label": "Speak instead of typing" }, svg("mic", 18));
  b.addEventListener("mousedown", (e) => e.preventDefault());
  let timer = null, started = 0;
  const idle = () => {
    clearInterval(timer);
    b.classList.remove("recording", "busy");
    b.disabled = false;
    b.replaceChildren(svg("mic", 18));
    b.setAttribute("aria-label", "Speak instead of typing");
    if (voiceRecording === b) voiceRecording = null;
  };
  const stop = async (auto) => {
    if (!b.classList.contains("recording")) return;
    clearInterval(timer);
    b.classList.replace("recording", "busy");
    b.disabled = true;
    b.replaceChildren(el("span", { class: "spinner" }));
    b.setAttribute("aria-label", "Writing it down");
    if (auto) stoppedSignal();
    let words = "";
    try {
      words = (await invoke("dictate_stop", { languages: note?.languages ?? null })).trim();
    } catch (e) {
      micToast(String(e));
    }
    idle();
    if (!words || !field.isConnected) return;
    const at = field.selectionStart ?? field.value.length, end = field.selectionEnd ?? at;
    const before = field.value.slice(0, at), after = field.value.slice(end);
    const text = (before && !/\s$/.test(before) ? " " : "") + words + (after && !/^\s/.test(after) ? " " : "");
    field.value = before + text + after;
    field.selectionStart = field.selectionEnd = at + text.length;
    field.dispatchEvent(new Event("input", { bubbles: true }));
    field.focus({ preventScroll: true });
    onWords?.(words);
  };
  b.addEventListener("click", async () => {
    if (b.classList.contains("recording")) return stop(false);
    if (b.disabled || dictation || voiceRecording) return;
    voiceRecording = b;
    b.disabled = true;
    try {
      await invoke("dictate_start");
    } catch (e) {
      idle();
      return showDictationError(e);
    }
    b.disabled = false;
    started = Date.now();
    b.classList.add("recording");
    b.setAttribute("aria-label", "Stop and write it down");
    label.textContent = "0:00";
    b.replaceChildren(el("span", { class: "rec-dot" }), label);
    timer = setInterval(() => {
      const s = (Date.now() - started) / 1000;
      label.textContent = clock(Math.min(s, VOICE_LIMIT_S));
      if (s >= VOICE_LIMIT_S || !b.isConnected) stop(b.isConnected);
    }, 250);
  });
  return b;
}

/// Dictation records until it's stopped, or until a silence after the person has finished. At each pause
/// the clip so far is saved and written down at the cursor while the next clip records, so nothing is missed.
async function startDictation() {
  const body = document.getElementById("note-body");
  if (!body || dictation || voiceRecording) return;
  dictation = { slots: new Map() };
  document.getElementById("dictation-chip").hidden = true;
  // Kept audio is filed under the note, so the note is saved as keeping it first.
  const keep = !!prefs.keep_audio && !!note;
  if (keep && (!note.id || !note.keepAudio)) {
    note.keepAudio = true;
    try {
      note.id = await invoke("save_note", noteArgs(note.id));
      view.id = note.id;
    } catch {}
  }
  const live = document.getElementById("dictating");
  const elapsed = el("span", { class: "left" }, "0:00");
  const wave = el("span", { class: "wave" }, Array.from({ length: 5 }, () => el("span")));
  const cancel = el("button", { class: "icon-btn cancel", title: "Stop (Esc). What isn't written yet is dropped, and the audio isn't kept.", "aria-label": "Cancel dictation", onclick: () => cancelDictation() }, svg("close", 18));
  const done = el("button", { class: "icon-btn keep", title: "Done (Ctrl + Space). It also stops by itself when you stop talking.", "aria-label": "Finish dictation", onclick: () => stopDictation() }, svg("check", 18));
  for (const b of [cancel, done]) b.addEventListener("mousedown", (e) => e.preventDefault());
  live.replaceChildren(el("span", { class: "rec" }, el("span", { class: "dot" })), el("span", {}, "Dictating"), wave, elapsed, cancel, done);
  live.hidden = false;
  document.getElementById("talk").hidden = true;
  // An invisible marker keeps the cursor's place: each clip is written in just before it, in order.
  dictation.anchor = placeDictationSlot();
  const started = Date.now();
  dictation.clockTimer = setInterval(() => { elapsed.textContent = clock((Date.now() - started) / 1000); }, 500);
  const d = dictation;
  try {
    d.session = await invoke("dictate_start", { continuous: true, note: keep ? note.id : null, languages: note?.languages ?? null });
  } catch (e) {
    d.failed = true;
    if (dictation !== d) return;
    removeDictationSlot(dictation.anchor);
    endDictationUi();
    dictation = null;
    showDictationError(e);
    return;
  }
  if (!dictation || dictation.handedOff) return;
  syncWake().catch(() => {});
  const bars = [...wave.children];
  let ticks = 0;
  dictation.levelTimer = setInterval(async () => {
    // Once a second, whether the system is giving the microphone silence, which is said rather than left
    // to look like a quiet room.
    if (platform === "android" && ++ticks % 10 === 0) {
      const problem = await invoke("dictation_problem").catch(() => null);
      const switched = await invoke("dictation_notice").catch(() => null);
      if (switched && dictation && switched[0] > (dictation.noticeId ?? 0)) {
        dictation.noticeId = switched[0];
        micToast(switched[1]);
      }
      const chip = document.getElementById("dictation-chip");
      if (problem && chip && chip.hidden) {
        chip.hidden = false;
        chip.replaceChildren(el("span", { class: "danger" }, problem), micHelp(problem));
      }
    }
    const level = await invoke("listen_level").catch(() => 0);
    bars.forEach((b, i) => { b.style.height = `${4 + Math.round(level * 16 * (0.6 + 0.4 * Math.sin(Date.now() / 90 + i)))}px`; });
  }, 100);
}

function endDictationUi() {
  clearInterval(dictation?.levelTimer);
  clearInterval(dictation?.clockTimer);
  const live = document.getElementById("dictating");
  if (live) live.hidden = true;
  const talk = document.getElementById("talk");
  if (talk) talk.hidden = false;
}

/// Recording has ended: once every clip's words are in, the marker goes. A clip still being written keeps
/// its widget until its words arrive, or for half a minute at most.
function finishDictation() {
  if (!dictation) return;
  dictation.finished = true;
  syncWake().catch(() => {});
  const mine = dictation;
  setTimeout(() => { if (dictation === mine) { for (const slot of mine.slots.values()) slot.remove(); mine.slots.clear(); tidyDictation(); } }, 30000);
  tidyDictation();
}

function tidyDictation() {
  if (!dictation || dictation.slots.size) return;
  const chip = document.getElementById("dictation-chip");
  if (dictation.wrote) dictation.anchor?.remove();
  else removeDictationSlot(dictation.anchor);
  if (!dictation.wrote && chip) { chip.hidden = false; chip.textContent = "No speech heard."; setTimeout(() => (chip.hidden = true), 2500); }
  dictation = null;
}

/// Stops dictating: the last clip gets its widget and is written in like the others. `byVoice` when ended
/// with "Hey Superteem, stop recording", which is then left out of the words.
async function stopDictation(byVoice = false) {
  if (!dictation || dictation.busy) return;
  dictation.busy = true;
  endDictationUi();
  const last = clipSlot();
  try {
    const heard = dictation.session ? await invoke("dictate_stop", { session: dictation.session, note: null }) : "";
    fillSlot(last, byVoice ? withoutStopCommand(heard) : heard);
  } catch (e) {
    last.remove();
    showDictationError(e);
  }
  finishDictation();
}

/// The words of a dictation without the "Hey Superteem, stop recording" that ended it.
function withoutStopCommand(words) {
  const wake = new RegExp(WAKE.source, "gi");
  let last = null;
  for (const m of words.matchAll(wake)) last = m;
  if (last && STOP_WORDS.test(words.slice(last.index + last[0].length))) return words.slice(0, last.index).trim();
  return words.replace(/[\s,.!]*\b(stop|end|finish)\b[\w\s']*\b(record\w*|note|dictat\w*)\W*$/i, "").trim();
}

/// The note was left while dictating (any way out of it): recording stops and what was said, clips still
/// being written down included, goes onto the end of the note in the background, so it's there on return.
let dictationHandoff = Promise.resolve();
function handOffDictation() {
  if (!dictation || !note) return;
  const d = dictation;
  dictation = null;
  d.handedOff = true;
  clearInterval(d.levelTimer);
  clearInterval(d.clockTimer);
  for (const slot of d.slots.values()) slot.remove();
  if (d.wrote) d.anchor?.remove();
  else removeDictationSlot(d.anchor);
  syncWake().catch(() => {});
  // Saved now, with the words already written in, before the next page takes `note`'s place.
  const args = noteArgs(note.id);
  const save = note.id && !note.dirty ? Promise.resolve(note.id) : invoke("save_note", args);
  note.dirty = false;
  clearTimeout(noteTimer);
  dictationHandoff = (async () => {
    // Started but not yet given its number: waits for it.
    while (!d.session && !d.failed) await new Promise((r) => setTimeout(r, 50));
    if (!d.session) return;
    const id = await save.catch(() => null);
    if (id) await invoke("dictate_detach", { session: d.session, note: id }).catch(() => {});
    else await invoke("dictate_cancel", { session: d.session }).catch(() => {});
    refreshItems();
  })();
}

/// Stops dictating and drops what isn't written in yet; words already written stay, to edit or undo.
async function cancelDictation() {
  if (!dictation || dictation.busy) return;
  dictation.busy = true;
  endDictationUi();
  if (dictation.session) await invoke("dictate_cancel", { session: dictation.session }).catch(() => {});
  for (const slot of dictation.slots.values()) slot.remove();
  if (dictation.wrote) dictation.anchor?.remove();
  else removeDictationSlot(dictation.anchor);
  dictation = null;
  syncWake().catch(() => {});
  document.getElementById("note-body")?.focus();
}

function showDictationError(e) {
  const chip = document.getElementById("dictation-chip");
  if (!chip) return;
  chip.hidden = false;
  chip.replaceChildren(el("span", { class: "danger" }, String(e)), " ",
    String(e).includes("model") ? el("button", { class: "pill", onclick: () => showTranscription() }, "Open Transcription settings") : micHelp(e));
}

// Ctrl + Space starts dictation in a note, and finishes it; Esc cancels it.
window.addEventListener("keydown", (e) => {
  if (view.kind !== "note" || isPhone()) return;
  if (e.code === "Space" && e.ctrlKey) {
    e.preventDefault();
    if (!e.repeat) dictation ? stopDictation() : startDictation();
  } else if (e.key === "Escape" && dictation && !document.querySelector("dialog[open]")) {
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
  const testing = !isPhone();
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
      : "A Bluetooth headset can be used once it's connected, even while this page is open. The phone may not record its own mic and a headset at the same time."),
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
  let sources = null;
  const update = () => {
    start.disabled = !(pins.checked || boxes.some((b) => b.box.checked) || sources?.chosen().length);
  };
  if (platform === "windows") {
    sources = soundSources(update);
    page.append(sources.section);
  }
  [pins, ...boxes.map((b) => b.box)].forEach((b) => b.addEventListener("change", update));
  update();
  start.addEventListener("click", async () => {
    try {
      const chosen = boxes.filter((b) => b.box.checked).map((b) => b.name).concat(sources?.chosen() ?? []);
      await startMeeting(chosen, pins.checked);
    } catch (e) {
      showError(page, e);
    }
  });
  page.append(el("div", { class: "actions" }, start));
  main.replaceChildren(page);
  const live = boxes.filter((b) => b.connected);
  // Android listens to its microphones too, so their meters show they work before recording starts.
  if (!live.length) return;

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
      if (p.state === "failed" || p.state === "absent") {
        b.box.disabled = true;
        // Ticked, it is ticked again if it comes back while the page is open.
        if (b.box.checked) { b.box.checked = false; b.untickedAway = true; update(); }
        // A headset that isn't connected is shown as any missing microphone is, not as a fault.
        b.row.className = p.state === "absent" ? "mic-row absent" : "mic-row failed";
        b.state.textContent = p.state === "absent" ? "Not connected" : `Not working: ${p.detail}`;
      } else if (p.state === "listening") {
        // Connected again (or for the first time) while the page is open: usable as a track.
        if (b.box.disabled) {
          b.box.disabled = false;
          if (b.untickedAway) { b.box.checked = true; b.untickedAway = false; update(); }
        }
        b.row.className = "mic-row working";
        b.state.replaceChildren(svg("check", 14), p.level > 0.3 ? " Hearing you" : " Connected");
      }
    }
  }, 150);
}

/// Sound from another app or an output device rather than a microphone (sources.rs names them).
const isSoundSource = (name) => name.startsWith("Sound from ") || name.startsWith("Everything playing on ");

/// "Sound from other apps" on the meeting page: apps appear as soon as they start playing (a Teams call,
/// a video), each with a meter, and ticked ones are recorded as tracks of their own. Windows only.
function soundSources(update) {
  const ticked = new Set((prefs.inputs ?? []).filter(isSoundSource));
  const rows = new Map();
  const list = el("div", { class: "mic-list" });
  const empty = el("div", { class: "empty" }, "Looking for apps playing sound…");
  const summary = el("summary", {}, "Everything on an output device");
  const others = el("details", { class: "hidden-mics" }, summary);
  // Outputs ticked are always in sight, so nothing is recorded that the page doesn't show.
  const showTicked = () => {
    const n = [...ticked].filter((t) => t.startsWith("Everything playing on ")).length;
    summary.textContent = n ? `Everything on an output device (${n} ticked)` : "Everything on an output device";
    if (n) others.open = true;
  };
  const othersList = el("div", { class: "mic-list" });
  others.append(othersList);
  const section = el("div", {},
    el("div", { class: "section-head" }, el("h2", {}, "Sound from other apps")),
    el("div", { class: "legend" }, "Record what other people say in a call: tick Teams, Zoom or your browser and their sound is recorded as its own track, beside your microphone. Apps appear here as soon as they play sound. Tick the app rather than your headset as well, or the call is recorded twice. Use a headset, or the call is also picked up by your microphone. Let people know they're being recorded."),
    list, empty, others);
  const row = (s) => {
    const box = el("input", { type: "checkbox", "aria-label": s.label });
    box.checked = ticked.has(s.name);
    box.addEventListener("change", () => { box.checked ? ticked.add(s.name) : ticked.delete(s.name); showTicked(); update(); });
    const meter = el("div", { style: "width:0%" });
    const state = el("span", { class: "mic-state" });
    const r = { box, meter, state, el: el("label", { class: "mic-row" }, box, el("span", { class: "mic-name" }, s.label, el("span", { class: "meter" }, meter)), state) };
    rows.set(s.name, r);
    (s.kind === "app" ? list : othersList).append(r.el);
    return r;
  };
  // Ticked last time but not playing yet: listed so it stays ticked, and recorded once it starts.
  for (const name of ticked) {
    if (name.startsWith("Sound from ")) row({ name, label: name.slice("Sound from ".length).replace(/ \([^)]*\)$/, ""), kind: "app" });
  }
  const refresh = async () => {
    const sources = await invoke("list_sound_sources").catch(() => null);
    if (!sources) return;
    const seen = new Set(sources.map((s) => s.name));
    for (const s of sources) {
      const r = rows.get(s.name) ?? row(s);
      r.meter.style.width = `${Math.round(Math.min(1, s.level * 1.5) * 100)}%`;
      r.el.className = "mic-row" + (s.active ? " working" : "");
      if (s.active) r.state.replaceChildren(svg("check", 14), " Playing");
      else r.state.textContent = s.kind === "app" ? "Quiet" : "";
    }
    for (const [name, r] of rows) {
      if (!seen.has(name)) { r.el.className = "mic-row absent"; r.state.textContent = r.box.checked ? "Recorded once it plays" : "Not playing"; }
    }
    showTicked();
    empty.hidden = list.children.length > 0;
    if (empty.textContent.startsWith("Looking")) empty.textContent = "No apps are playing sound. Start the call and it appears here.";
  };
  refresh();
  clearInterval(sourceTimer);
  sourceTimer = setInterval(refresh, 400);
  return { section, chosen: () => [...ticked].filter((n) => rows.has(n)) };
}

/// Microphones heard working before, from a test, a voice print or a recording, that aren't hidden.
function knownMics(connected) {
  const worked = new Set(prefs.working_mics ?? []);
  if (prefs.enrol_input) worked.add(prefs.enrol_input);
  for (const [name, r] of Object.entries(prefs.mics ?? {})) if (r.ok && r.last_ok >= r.last_failed) worked.add(name);
  const hidden = new Set(prefs.hidden_mics ?? []);
  // Connected ones first, so the usable tracks are at the top.
  return [...worked].filter((m) => !hidden.has(m) && m !== "Superteem pins" && !isSoundSource(m))
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
    fromTracks ? el("button", { class: "link back-link", onclick: () => showMeeting() }, "‹ Back to the meeting")
      : el("button", { class: "link back-link", onclick: () => showVoices() }, "‹ Voices"),
    el("div", { class: "section-head" }, el("h1", {}, "Devices"),
      el("button", { class: "pill", onclick: () => showDevices(fromTracks) }, "Refresh devices")),
    el("p", { class: "legend" }, "Say something to test each microphone. Once it hears you it shows ✓ and can be used as a meeting track. Hide any you never use or that don't work."),
    el("h2", {}, "Microphones"),
    el("div", { class: "mic-list" }, rows.map((r) => r.row),
      shown.length ? null : el("div", { class: "empty" }, "No microphones to show.")),
    el("div", { class: "legend" }, `To add a microphone, plug it in or pair it in ${platform === "macos" ? "the Mac's" : "Windows"} Bluetooth settings, then press Refresh devices.`),
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
  clearInterval(sourceTimer);
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

/// A track's tooltip: where its sound came from and what happened while recording.
function trackDetails(name, t) {
  if (!t) return name;
  const kind = { app: "App sound", output: "Everything on an output", microphone: "Microphone", pin: "Voice Pin" }[t.kind] ?? t.kind;
  return [`${name} · ${kind}`, t.source !== name ? t.source : "", t.peak === 0 ? "Silent" : "", ...(t.notes ?? [])].filter(Boolean).join("\n");
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

/// `kind` is "action" for an action item; a bookmark has none.
function addMarker(text, kind = "") {
  if (!recording) return false;
  recording.markers.push({ at_s: Math.round(nowInRecording() * 10) / 10, text, ...(kind ? { kind } : {}) });
  drawMarkers();
  return true;
}

/// Markers saved before action items had a kind were noted as text.
const isAction = (m) => m.kind === "action" || (!m.kind && m.text === "Action item");
const markerLabel = (m) => (isAction(m) ? "Action item" : "Bookmark");
/// A marker's icon in place of its name, which screen readers and the tooltip still give.
const markerIcon = (m, size = 16) => {
  const icon = el("span", { class: `marker-icon${isAction(m) ? " action" : ""}`, role: "img", "aria-label": markerLabel(m), title: markerLabel(m) });
  icon.innerHTML = svg(isAction(m) ? "action" : "bookmark", size).outerHTML;
  return icon;
};

/// The marker open for changing while recording.
let editingMarker = null;
/// Marks now on the recording page, with whatever note is typed: set while that page is drawn.
let liveMark = null;

/// Every marker made so far; clicking one opens it to change its note or delete it.
function drawMarkers() {
  const list = document.getElementById("live-markers");
  if (!list || !recording) return;
  if (!recording.markers.includes(editingMarker)) editingMarker = null;
  const typed = (m) => (m.text && m.text !== "Action item" ? m.text : "");
  list.replaceChildren(...recording.markers.map((m) => {
    const what = markerLabel(m).toLowerCase();
    if (m === editingMarker) {
      const input = el("input", { value: typed(m), placeholder: isAction(m) ? "What needs doing" : "Add a note to this bookmark", "aria-label": `Note for the ${what} at ${clock(m.at_s)}` });
      const close = () => { editingMarker = null; drawMarkers(); };
      const done = () => { if (editingMarker === m) { m.text = input.value.trim(); close(); } };
      input.addEventListener("keydown", (e) => {
        if (e.key === "Enter") { e.preventDefault(); e.stopPropagation(); done(); }
        if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); close(); }
      });
      const remove = () => { recording.markers.splice(recording.markers.indexOf(m), 1); close(); };
      const row = el("div", { class: `marker editing${isAction(m) ? " action" : ""}` }, el("span", { class: "mono" }, clock(m.at_s)),
        markerIcon(m), input,
        el("button", { class: "icon-btn save", type: "button", title: "Save (Enter)", "aria-label": "Save", onclick: done }, svg("save", 18)),
        el("button", { class: "icon-btn trash", type: "button", title: `Delete this ${what}`, "aria-label": `Delete this ${what}`, onclick: remove }, svg("trash", 18)));
      // Clicking anywhere else keeps what was typed; Esc leaves it as it was.
      row.addEventListener("focusout", (e) => { if (!row.contains(e.relatedTarget)) done(); });
      requestAnimationFrame(() => input.focus());
      return row;
    }
    const open = () => { editingMarker = m; drawMarkers(); };
    const row = el("div", { class: `marker${isAction(m) ? " action" : ""}`, role: "button", tabindex: "0", title: "Click to change the note or delete it" },
      el("span", { class: "mono" }, clock(m.at_s)), markerIcon(m), typed(m) ? el("span", {}, typed(m)) : el("span", { class: "hint" }, isAction(m) ? "Action item" : "Bookmark"));
    row.addEventListener("click", open);
    row.addEventListener("keydown", (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); open(); } });
    return row;
  }));
  if (!editingMarker) list.scrollTop = list.scrollHeight;
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
  const problem = el("div", { "aria-live": "polite" });
  // Press the bookmark or Enter to mark now, or the tick for an action item, with whatever note is typed.
  const pinned = el("input", { placeholder: "Mark now, with a note if you like: Enter bookmarks it", "aria-label": "Note pinned to the current time" });
  const at = el("span", { class: "mono" }, `@${clock(status.elapsed_s)}`);
  const mark = (kind) => {
    addMarker(pinned.value.trim(), kind);
    pinned.value = "";
  };
  liveMark = mark;
  pinned.addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); mark(e.ctrlKey ? "action" : ""); } });
  const bookmark = el("button", { class: "mark-btn bookmark", type: "button", "aria-label": "Bookmark this moment", title: "Bookmark this moment (B, or Enter in the note)", onclick: () => mark("") },
    svg("bookmark", 18), el("span", { class: "label" }, "Bookmark"), el("kbd", {}, "B"));
  const actionItem = el("button", { class: "mark-btn action-item", type: "button", "aria-label": "Add an action item", title: "Add an action item (A, or Ctrl + Enter in the note). Type what needs doing, or add it afterwards.", onclick: () => mark("action") },
    svg("action", 18), el("span", { class: "label" }, "Action item"), el("kbd", {}, "A"));
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
    problem, time, lanes, el("div", { class: "markers", id: "live-markers" }),
    el("div", { class: "mark-buttons" }, bookmark, actionItem),
    el("div", { class: "pinned" }, at, pinned),
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
      const failing = ["failed", "disconnected", "silenced", "no headset"].includes(st.state);
      return el("div", { class: "lane" },
        el("span", { class: "name", style: `color:${colour}` }, streamName(st.name),
          el("small", { class: failing ? "danger" : "" }, failing || st.state === "reconnecting" || st.state === "waiting" ? st.detail || st.state : st.state)),
        el("span", { class: "spark" }, history.map((l) => el("span", { style: `height:${Math.max(2, Math.round(l * 24))}px;background:${l > 0.35 ? colour : "var(--line)"}` }))));
    }));
    if (!s.streams.length) lanes.replaceChildren(el("div", { class: "legend" }, "Starting the inputs…"));
    // Android switched a stream to another microphone: said briefly, once per switch.
    recording.notices ??= {};
    for (const st of s.streams) {
      if (st.notice_id > (recording.notices[st.name] ?? 0)) {
        recording.notices[st.name] = st.notice_id;
        micToast(st.notice);
      }
    }
    // Every microphone silent because of the system, not the room: say so, with the way to fix it.
    const blocked = s.streams.length && s.streams.every((st) => ["failed", "silenced", "no headset"].includes(st.state));
    if (!blocked) problem.replaceChildren();
    else if (!problem.firstChild) {
      problem.replaceChildren(el("div", { class: "notice", role: "alert" },
        el("strong", {}, "Nothing is being recorded. "), s.streams[0].detail || "The microphone isn't working.", micHelp("microphone")));
    }
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

/// Plays every track together. A track moved since the meeting was transcribed plays `shift` seconds later
/// than its file has it (earlier when negative), so a move is heard at once.
function makePlayer(session, duration, onTick) {
  const audios = new Map();
  const shifts = new Map(Object.entries(session.pending_offsets ?? {}));
  for (const [stream, path] of Object.entries(session.audio)) {
    const audio = new Audio(convertFileSrc(path));
    audio.preload = "auto";
    audios.set(stream, audio);
  }
  const p = { audios, shifts, t: 0, playing: false, timer: null, duration };
  // The clock: an unmoved track while it plays, else the time since play was pressed.
  const steady = [...audios.entries()].filter(([s]) => !shifts.get(s)).map(([, a]) => a);
  let started = null;
  const place = (stream, a, resync) => {
    const own = p.t - (shifts.get(stream) ?? 0);
    if (own < 0 || (a.duration && own >= a.duration)) {
      // Before it starts or after it ends here: silent until the playhead reaches it.
      if (!a.paused) a.pause();
      if (own < 0 && a.currentTime) a.currentTime = 0;
      return;
    }
    if (resync || Math.abs(a.currentTime - own) > 0.25) a.currentTime = own;
    if (p.playing && a.paused) a.play().catch(() => {});
  };
  p.seek = (t) => {
    p.t = Math.max(0, Math.min(duration, t));
    started = { t: p.t, at: performance.now() };
    for (const [s, a] of audios) place(s, a, true);
    onTick(p.t, p.playing);
  };
  p.play = () => {
    if (!audios.size) return;
    p.playing = true;
    started = { t: p.t, at: performance.now() };
    for (const [s, a] of audios) place(s, a, true);
    clearInterval(p.timer);
    p.timer = setInterval(() => {
      const clock = steady.find((a) => !a.paused && !a.ended);
      p.t = clock ? clock.currentTime : started.t + (performance.now() - started.at) / 1000;
      if (clock) started = { t: p.t, at: performance.now() };
      if (p.t >= duration) return p.pause();
      for (const [s, a] of audios) if (a !== clock) place(s, a, false);
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

/// "142 MB", for space on disk.
function sizeText(bytes) {
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  if (bytes < 1024 ** 3) return `${Math.round(bytes / 1024 ** 2)} MB`;
  return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
}

/// The bin button's menu: [choice, label, detail, disabled] each.
/// A "more" (three dots) button and the actions it opens. `always` keeps them behind the button at every
/// width; otherwise they sit side by side on a computer and go behind it only on a phone.
function moreMenu(items, { always = false } = {}) {
  const button = el("button", { class: "icon-only top-more-button", type: "button", title: "More", "aria-label": "More actions", "aria-haspopup": "menu", "aria-expanded": "false" }, svg("more", 18));
  const list = el("span", { class: "top-more-items" }, items.filter(Boolean));
  const more = el("span", { class: "top-more" + (always ? " always" : "") }, button, list);
  const close = () => { more.classList.remove("open"); button.setAttribute("aria-expanded", "false"); document.removeEventListener("click", outside, true); };
  const outside = (e) => { if (!more.contains(e.target)) close(); };
  button.addEventListener("click", () => {
    if (more.classList.contains("open")) return close();
    more.classList.add("open");
    button.setAttribute("aria-expanded", "true");
    document.addEventListener("click", outside, true);
  });
  // Choosing from a submenu, or a plain action, is the end of it; a note's Delete stays, to be pressed again.
  list.addEventListener("click", (e) => { if (e.target.closest(".export-menu .menu, .more-action")) close(); });
  return more;
}

function deleteMenu(choices, run) {
  const wrap = el("span", { class: "export-menu delete-menu" });
  const button = el("button", { class: "icon-only danger", type: "button", "aria-haspopup": "menu", "aria-expanded": "false",
    title: "Delete this meeting or its audio", "aria-label": "Delete this meeting or its audio" }, svg("trash", 18), el("span", { class: "menu-text" }, "Delete"));
  const menu = el("div", { class: "menu", role: "menu", hidden: true });
  const close = () => { menu.hidden = true; button.setAttribute("aria-expanded", "false"); document.removeEventListener("click", outside, true); };
  const outside = (e) => { if (!wrap.contains(e.target)) close(); };
  button.addEventListener("click", () => {
    if (!menu.hidden) return close();
    menu.hidden = false;
    button.setAttribute("aria-expanded", "true");
    document.addEventListener("click", outside, true);
    menu.querySelector("button:not([disabled])")?.focus();
  });
  menu.addEventListener("keydown", (e) => { if (e.key === "Escape") { close(); button.focus(); } });
  for (const [choice, label, detail, disabled] of choices) {
    const item = el("button", { type: "button", role: "menuitem", disabled }, el("strong", {}, label), el("small", {}, detail));
    item.addEventListener("click", () => { close(); run(choice); });
    menu.append(item);
  }
  wrap.append(button, menu);
  return wrap;
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
    [session] = await Promise.all([invoke("get_session", { id }), languageData()]);
  } catch (e) {
    main.replaceChildren(el("div", { class: "page" }, el("div", { class: "error" }, String(e))));
    return;
  }
  renderSession(session, processingError);
}

let sessionListeners = null;
/// Whether the meeting page also shows the clips left out of the script as doubles.
let showDoubles = false;
/// Whether the meeting page's "?" notes are open.
let helpOpen = false;
/// What a meeting plays: "transcript", with each track muted where the script takes its words from another
/// microphone, or "manual", every track unless muted by hand.
let playMix = "transcript";

/// `suggestion` is lines that sound like one just moved: { lines, speaker }.
function renderSession(session, processingError, suggestion) {
  const { id, meta } = session;
  // Silence and noise have no words; only people talking are shown. Meetings processed before
  // silence was dropped still have these lines.
  if (session.transcribed) session.segments = session.segments.filter((s) => s.edited || hasWords(s.text));
  window.currentPrints = session.voiceprints;
  // Clips another microphone heard more clearly are left out of the script; they stay, greyed, in the tracks.
  const doubles = session.segments.filter((s) => s.echo_of != null);
  const scriptLines = session.segments.filter((s) => s.echo_of == null);
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

  // Delete offers the whole meeting, or just its audio ("Compact"), which keeps the transcript.
  const confirmBox = el("div", { class: "notice confirm", role: "alertdialog", hidden: true });
  const askFirst = (text, label, run) => {
    const go = el("button", { class: "primary danger-fill", type: "button" }, label);
    go.addEventListener("click", async () => { go.disabled = true; await run(); go.disabled = false; });
    confirmBox.replaceChildren(el("p", {}, text), go,
      el("button", { type: "button", onclick: () => { confirmBox.hidden = true; } }, "Cancel"));
    confirmBox.hidden = false;
    go.focus();
  };
  const deleteWhole = async () => {
    stopPlayback();
    if (await save("delete_session", {})) showHome();
  };
  const compactWhy = session.busy ? "Not while it is being recorded or processed"
    : session.needs_processing ? "Not processed yet, so there is no transcript to keep"
    : meta.compacted || !session.audio_bytes ? "Its audio is already deleted" : null;
  const compact = async () => {
    stopPlayback();
    try {
      const freed = await invoke("compact_session", { id });
      await reload();
      const notice = main.querySelector(".compacted-notice");
      if (notice && freed) notice.prepend(`Audio deleted, ${sizeText(freed)} freed. `);
    } catch (e) {
      showError(page, e);
    }
  };
  const remove = deleteMenu([
    ["delete", isPhone() ? "Delete permanently" : `Move to ${binName()}`,
      isPhone() ? "The whole meeting, transcript and audio" : `The whole meeting. It can be restored from the ${binName()}.`],
    ["compact", "Compact", compactWhy ?? `Delete only the audio and keep the transcript · frees ${sizeText(session.audio_bytes)}`, !!compactWhy],
  ], (choice) => {
    if (choice === "compact") {
      askFirst(`Delete this meeting's audio for good and free ${sizeText(session.audio_bytes)}? The transcript, names, bookmarks and action items stay and can still be edited and exported, `
        + "but the meeting can't be played or transcribed again. This can't be undone.", "Delete audio", compact);
    } else if (isPhone()) {
      askFirst("Delete this meeting and its audio for good? This can't be undone.", "Delete", deleteWhole);
    } else {
      deleteWhole();
    }
  });
  const edits = session.segments.filter((s) => s.edited).length;
  const reprocess = el("button", { class: "icon-only more-action", type: "button", title: "Transcribe again: run speaker detection and transcription again", "aria-label": "Transcribe again" },
    svg("refresh", 18), el("span", { class: "menu-text" }, "Transcribe again"));
  reprocess.addEventListener("click", () => processAgain(id));
  // A track's own settings, opened from its name: who is on it, whether everything on it is them, and
  // where it starts. A move takes effect when the meeting is transcribed again.
  const trackDialog = (stream, name) => {
    const speaker = session.speakers.find((s) => s.key === stream);
    const imported = session.imports?.find((i) => i.name === stream);
    const group = imported?.from_session ? session.imports.filter((i) => i.from_session === imported.from_session) : [];
    const offset = imported ? imported.offset_s : meta.track_offsets?.[stream] ?? 0;
    const locked = (meta.locked ?? []).includes(stream);
    const nameInput = el("input", { type: "text", value: speaker?.name ?? name, maxlength: "60", "aria-label": "Name" });
    const print = el("select", { "aria-label": "Voice print" },
      el("option", { value: "" }, "None"),
      ...(session.voiceprints ?? []).map((p) => el("option", { value: p.id, selected: speaker?.print?.id === p.id }, p.name)));
    // A recognised print is linked to the person's own print id, which may not be the one listed for them.
    if (speaker?.print && ![...print.options].some((o) => o.value === speaker.print.id)) {
      print.append(el("option", { value: speaker.print.id, selected: true }, speaker.print.name));
    }
    const original = print.value;
    print.addEventListener("change", () => {
      const chosen = print.selectedOptions[0];
      if (print.value) nameInput.value = chosen.textContent;
    });
    const lock = el("input", { type: "checkbox", checked: locked });
    const start = el("input", { type: "text", class: "mono", size: "9", value: signedClock(offset).replace("−", "-"), "aria-label": "Starts at" });
    const status = el("div", { class: "hint", role: "status" });
    const fail = (e) => status.replaceChildren(el("span", { class: "danger" }, String(e?.message ?? e)));
    // Bookmarks made in an imported session line it up with the same moment marked here.
    let lineUp = null;
    let lined = false;
    const theirs = imported?.markers ?? [];
    const mine = allMarkers.filter((m) => !m.removed);
    if (theirs.length && mine.length) {
      const label = (m, at) => `${markerLabel(m)}${markerNote(m) ? ` “${markerNote(m)}”` : ""} · ${clock(at)}`;
      const theirSelect = el("select", { "aria-label": "Their bookmark" }, theirs.map((m, k) => el("option", { value: k }, label(m, m.at_s))));
      const mineSelect = el("select", { "aria-label": "Your bookmark" }, mine.map((m, k) => el("option", { value: k }, label(m, m.at_s))));
      // The same note in both, such as "Sync", is the likely pair.
      const same = (a, b) => (markerNote(a) || "").trim().toLowerCase() === (markerNote(b) || "").trim().toLowerCase();
      const pair = theirs.findIndex((t) => markerNote(t) && mine.some((m) => same(t, m)));
      if (pair >= 0) {
        theirSelect.value = String(pair);
        mineSelect.value = String(mine.findIndex((m) => same(theirs[pair], m)));
      }
      const go = el("button", { class: "pill", type: "button" }, "Line up");
      go.addEventListener("click", async () => {
        go.disabled = true;
        try {
          const moved = await invoke("line_up_track", { id, name: stream, ownS: theirs[theirSelect.value].at_s, meetingS: mine[mineSelect.value].at_s });
          start.value = signedClock(moved.offset_s).replace("−", "-");
          status.replaceChildren(`Lined up: it now starts ${signedClock(moved.offset_s)} into the meeting. Close this to hear it there; its lines in the transcript follow when you transcribe again.`);
          lined = true;
        } catch (e) { fail(e); }
        go.disabled = false;
      });
      lineUp = el("div", { class: "field" }, el("span", {}, "Line up by a bookmark",
        el("small", {}, "Their bookmark is the same moment as yours, such as one everyone made at once.")),
        el("div", { class: "row-pick" }, theirSelect, el("span", { class: "hint" }, "is"), mineSelect, go));
    }
    const whose = group.length > 1 ? ` Moves all ${group.length} tracks recorded with it.` : "";
    const saveButton = el("button", { class: "primary pill", type: "button" }, "Save");
    const dialog = el("dialog", { class: "sheet track-dialog", "aria-label": `Track ${name}` },
      el("h2", {}, name),
      el("p", { class: "hint source" }, imported ? `Imported · ${placedText(imported, meta.created)}` : trackDetails(name, meta.tracks?.[stream]).split("\n").slice(0, 2).join(" · ")),
      el("label", { class: "field" }, el("span", {}, "Name", el("small", {}, "Who is on this track, or what it recorded.")), nameInput),
      el("label", { class: "field" }, el("span", {}, "Voice print", el("small", {}, "Links the person, so the voice print names them.")), print),
      el("label", { class: "field check" }, lock, el("span", {}, "Everything on this track is this person",
        el("small", {}, "For when each person has their own microphone or call track. Lines you moved by hand stay put."))),
      el("label", { class: "field" }, el("span", {}, "Starts at",
        el("small", {}, `minutes:seconds into the meeting; − if it starts before. You hear a move straight away; its lines in the transcript follow when you transcribe again.${whose}`)), start),
      lineUp, status,
      el("div", { class: "actions" }, el("button", { class: "pill", type: "button", onclick: () => dialog.close() }, "Cancel"), saveButton));
    saveButton.addEventListener("click", async () => {
      const at = parseStart(start.value);
      if (at == null) return fail("Type where it starts as minutes:seconds, such as 0:01.5 or -0:00.4.");
      saveButton.disabled = true;
      try {
        if (print.value !== original) await invoke("link_speaker", { id, key: stream, print: print.value || null, learn: false });
        const typed = nameInput.value.trim();
        const printName = print.value ? print.selectedOptions[0].textContent : null;
        if (typed && typed !== (speaker?.name ?? name) && typed !== printName) await invoke("rename_speaker", { id, key: stream, name: typed });
        const moved = Math.abs(at - offset) > 0.004 && !lined;
        if (moved || lock.checked !== locked) {
          await invoke("set_track", { id, name: stream, offsetS: moved ? at : null, lock: lock.checked !== locked ? lock.checked : null });
        }
        lined = false;
        dialog.close();
        reload();
      } catch (e) {
        saveButton.disabled = false;
        fail(e);
      }
    });
    dialog.addEventListener("close", () => { dialog.remove(); if (lined) reload(); });
    document.body.append(dialog);
    dialog.showModal();
    nameInput.focus();
  };
  const recordedTrackMenu = (stream, name) => {
    const heard = scriptLines.filter((s) => s.stream === stream).length;
    const edited = session.segments.filter((s) => s.stream === stream && s.edited).length;
    const silent = meta.tracks?.[stream]?.peak === 0;
    return menuButton(svg("more", 14), [
      "This track",
      [isPhone() ? "Delete this track" : "Remove this track",
        silent ? "It heard nothing · transcribes the meeting again without it" : "Transcribes the meeting again without it",
        () => {
          // Asked in the tracks panel, where the menu was, so it's seen.
          tracks.querySelector(".track-confirm")?.remove();
          const what = heard ? ` After transcribing again, the ${heard} line${heard === 1 ? "" : "s"} taken from it will come from the other tracks, where they heard it.` : "";
          const lost = edited ? ` Your edits to ${edited} of its line${edited === 1 ? "" : "s"} will be lost.` : "";
          const where = isPhone() ? " This can't be undone." : ` Its audio goes to the ${binName()}.`;
          const go = el("button", { class: "primary danger-fill", type: "button" }, "Remove track");
          const box = el("div", { class: "inline-form track-confirm", role: "alertdialog" },
            el("p", {}, `Remove ${name} from this meeting?${what}${lost}${where}`), go,
            el("button", { class: "pill", type: "button", onclick: () => box.remove() }, "Cancel"));
          go.addEventListener("click", async () => {
            go.disabled = true;
            stopPlayback();
            try {
              await invoke("remove_track", { id, name: stream });
              await reload();
            } catch (e) { go.disabled = false; showError(box, e); }
          });
          tracks.prepend(box);
          go.focus();
        }],
    ]);
  };
  const used = new Set(scriptLines.map((s) => s.speaker));
  const streams = Object.keys(session.audio).sort();
  const picker = statusPicker(meta.status, async (status) => {
    if (!(await save("set_session_status", { status }))) return false;
    meta.status = status;
    refreshItems();
    return true;
  });
  // Fetched before the page is drawn (see openSession).
  let meetingLanguages = itemLanguages(meta);
  const meetingLanguage = () => meetingLanguages[0];
  const langNotice = el("div", { class: "notice language-notice", hidden: true, "aria-live": "polite" });
  const languageChoice = languagePicker(meetingLanguages, async (list) => {
    const result = await changeItemLanguages("meeting", id, list, langNotice);
    if (!result) return false;
    meetingLanguages = result.effective;
    meta.languages = result.languages;
    setLanguage([page], meetingLanguage());
    return true;
  });
  const exporter = exportMenu([...TRANSCRIPT_EXPORTS, ...(meta.compacted ? [] : [
    ["package", "Edit package", "Every track and the transcript mix as audio, with an EDL, Premiere and Resolve XML and a REAPER project"]])],
    (format) => invoke("export_session", { id, title: meetingTitle(id, meta.title), format }));
  // Like a note's: the status and the meeting's actions in a bar of their own across the top.
  const topBar = el("div", { class: "topbar meta meeting-top" },
    picker, languageChoice,
    // On a phone this drops to its own line under the status and the buttons.
    el("span", { class: "top-info" }, el("span", { title: dateText(meta.created) }, agoText(meta.created))),
    el("span", { class: "top-actions" },
    scriptLines.length ? readAloudButton(() => scriptLines.filter((l) => hasWords(l.text)).map((l) => (prefs.read_speakers ?? true ? `${session.speakers.find((sp) => sp.key === l.speaker)?.name ?? "Someone"}: ${l.text}` : l.text)).join("\n"), "Read the transcript aloud", { lang: meetingLanguage }) : null,
    isDesktop() ? el("button", { class: "icon-only", title: meta.compacted ? "Open this meeting's folder, with its transcript and exports" : "Open this meeting's folder, with its audio, transcript and exports",
      "aria-label": "Open this meeting's folder", onclick: () => invoke("open_folder", { id }) }, svg("folder", 18)) : null,
    // Export, Transcribe again and Delete sit behind one "more" button.
    moreMenu([session.segments.length ? exporter : null, session.model_ready && !session.needs_processing && !meta.compacted ? reprocess : null, remove], { always: true })));
  page.append(exporter.status, confirmBox, langNotice, title);

  const processNow = el("button", { class: "primary" }, "Process now");
  processNow.addEventListener("click", () => processAgain(id));
  if (meta.compacted) {
    page.append(el("div", { class: "notice compacted-notice" },
      "This meeting's audio was deleted to save space. The transcript can still be read, edited and exported, but not played or transcribed again."));
  } else if (session.needs_processing) {
    page.append(el("div", { class: "notice" },
      el("p", {}, "This recording was saved but not processed, perhaps because the app closed while recording. Its audio is safe."),
      processNow));
  } else if (processingError) {
    page.append(el("div", { class: "notice" }, `Processing failed: ${processingError}`));
  } else if (!session.segments.length) {
    page.append(el("div", { class: "notice" }, "No speech was found in this recording."));
  } else if (!session.transcribed && !meta.compacted) {
    page.append(el("div", { class: "notice" }, session.model_ready
      ? "This meeting has no transcript yet. Use Transcribe (↻) above."
      : "This meeting has no transcript. Get the speech model in Settings, then use Transcribe (↻) above."));
  }
  // Analyses sit under the title, above the transcript; their sources jump to the line and play from it.
  if (session.transcribed) {
    const goToLine = (source) => {
      const hit = lineEls.find((l) => l.seg.id === source.segment);
      if (hit) {
        hit.line.scrollIntoView({ behavior: "smooth", block: "center" });
        hit.line.classList.add("flash");
        setTimeout(() => hit.line.classList.remove("flash"), 1800);
      }
      if (player && source.at_s != null) player.seek(source.at_s);
    };
    page.append(analysisStack("meeting", id, {
      lang: meetingLanguage,
      speakers: session.speakers.filter((sp) => sp.key !== "unknown").map((sp) => sp.name),
      go: goToLine,
      // Translations into other languages, under the analyses.
      translations: translationStack("meeting", id, meetingLanguages, { go: (line) => goToLine({ segment: Number(line.id), at_s: line.at_s }) }),
    }));
  }
  // Shown behind the "?" with the other notes on reading the transcript.
  const extraHelp = [];
  if (edits) extraHelp.push(el("p", {}, `${edits} line${edits > 1 ? "s" : ""} edited by hand. Transcribing again keeps them.`));
  if (doubles.length) {
    const toggle = el("button", { class: "link inline", type: "button", "aria-pressed": String(showDoubles) }, showDoubles ? "Hide them" : "Show them");
    toggle.addEventListener("click", () => { showDoubles = !showDoubles; helpOpen = true; renderSession(session, processingError); });
    extraHelp.push(el("p", {},
      `${doubles.length} clip${doubles.length > 1 ? "s" : ""} heard again on another microphone ${doubles.length > 1 ? "are" : "is"} greyed out in the tracks and left out of the script. `, toggle));
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
      : sp.suggested.length ? el("span", { class: "badge", title: "Not sure who this is. Pick them below." }, "who?") : null;
    const menu = speakerMenu(sp, usedSpeakers, (action) => speakerAction(action, sp));
    speakerBar.append(el("span", { class: "chip", style: `background:${colours.get(sp.key)}` }, input, badge, menu));
  }
  if (used.size) {
    // How to read and fix the transcript, behind a "?" so it isn't in the way once known.
    const help = el("div", { class: "legend", hidden: !helpOpen, id: "transcript-help" }, el("p", {}, "Rename a voice to update every line, or use ⋯ to merge voices or link a voice print. "
      + "Dotted lines are less certain; a side bar means two people spoke at once."), extraHelp);
    const helpButton = el("button", { class: `icon-only help-toggle${helpOpen ? " on" : ""}`, type: "button", title: "How to read and fix the transcript",
      "aria-label": "How to read and fix the transcript", "aria-expanded": String(helpOpen), "aria-controls": "transcript-help" }, "?");
    helpButton.addEventListener("click", () => {
      help.hidden = !help.hidden;
      helpOpen = !help.hidden;
      helpButton.setAttribute("aria-expanded", String(!help.hidden));
      helpButton.classList.toggle("on", !help.hidden);
    });
    speakerBar.append(helpButton);
    // Speakers the voice prints weren't sure about: the people it might be, to pick from.
    const asks = usedSpeakers.filter((sp) => !sp.print && sp.suggested.length).map((sp) => {
      const choices = sp.suggested.map((p) => {
        const pick = el("button", { type: "button", title: `Sounds ${Math.round(p.score * 100)}% like ${p.name}'s voice print` },
          `${p.name} · ${Math.round(p.score * 100)}%`);
        pick.addEventListener("click", () => speakerAction(`link:${p.id}`, sp));
        return pick;
      });
      const none = el("button", { type: "button", class: "link" }, "None of these");
      none.addEventListener("click", async () => { if (await save("link_speaker", { key: sp.key, print: null, learn: false })) reload(); });
      const few = sp.voiced_s < 4 ? ` from only ${Math.max(1, Math.round(sp.voiced_s))} s of speech` : "";
      return el("div", { class: "inline-form" }, el("span", {}, el("strong", {}, `Who is ${sp.name}?`), ` Not sure${few}. It sounds most like:`),
        choices, none);
    });
    page.append(speakerBar, ...asks, speakerForm, help);
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
            target.replaceChildren(el("div", { class: "inline-form" }, "Done. They will be recognised better next time."));
          } catch (e) { showError(target, e); }
        });
        target.replaceChildren(el("div", { class: "inline-form" },
          el("span", { title: "If they sound unlike their voice prints here, as through a call app or another microphone, this becomes another voice print for them." },
            `Linked. Also learn how they sounded here (${Math.round(sp.voiced_s)} s of speech)?`),
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
  const trackName = (stream) => session.speakers.find((s) => s.key === stream)?.name ?? streamName(stream, session.meta.inputs?.length ? session.meta.inputs : prefs.inputs);
  const lineEls = [];
  const transcript = el("div", { class: "transcript" });
  const allMarkers = sessionMarkers(meta.markers ?? [], session.keyphrases ?? []);
  const markers = [...allMarkers];
  let previous = null;
  const playFrom = (t) => { player?.seek(t); player?.play(); };
  // Clicking a bookmark or action item, here or in the tracks, opens it for changing its note or deleting it.
  const markerEditors = new Map();
  const saveMarker = async (m, text, remove) => {
    if (await save("edit_marker", { index: m.index ?? null, atS: m.at_s, kind: m.kind ?? "", text, remove })) reload();
  };
  const markerLine = (m) => {
    const note = markerNote(m);
    const typed = note;
    const body = el("div", { class: "marker-body", role: "button", tabindex: "0", title: `${markerLabel(m)}: click to ${note ? "change the note" : "type a note"} or delete it` },
      markerIcon(m), note ? el("span", {}, note) : el("span", { class: "hint" }, isAction(m) ? "Action item: tap to say what needs doing" : markerLabel(m)),
      m.heard ? el("span", { class: "edited", title: "Heard in the recording when it was processed" }, "heard") : null);
    // An action item with no note can have a short one suggested, from what was said around it.
    let suggest = null;
    if (isAction(m) && !note) {
      const key = `${id}/${m.at_s}`;
      const busy = labelling.has(key);
      suggest = el("button", { class: "pill with-icon marker-suggest", type: "button", disabled: busy,
        title: "Have the language model write a short label from what was said here", "aria-label": "Suggest a label for this action item" },
        busy ? el("span", { class: "spinner" }) : svg("sparkle", 14), busy ? "Writing…" : "Suggest");
      suggest.addEventListener("click", async () => {
        labelling.add(key);
        suggest.disabled = true;
        suggest.replaceChildren(el("span", { class: "spinner" }), "Writing…");
        try {
          await invoke("label_action_item", { id, atS: m.at_s });
        } catch (e) {
          labelling.delete(key);
          suggest.disabled = false;
          suggest.replaceChildren(svg("sparkle", 14), "Suggest");
          showError(page, e);
        }
      });
    }
    const line = el("div", { class: `line marker-line${isAction(m) ? " action" : ""}` },
      el("button", { class: "time mono", title: "Play from here", onclick: () => playFrom(m.at_s) }, clock(m.at_s)),
      el("div", { class: "marker-row" }, body, suggest));
    const edit = () => {
      if (line.classList.contains("editing")) return;
      line.classList.add("editing");
      const input = el("input", { value: typed, placeholder: isAction(m) ? "What needs doing" : "Add a note to this bookmark",
        "aria-label": `Note for the ${markerLabel(m).toLowerCase()} at ${clock(m.at_s)}` });
      let closed = false;
      const cancel = () => { closed = true; line.classList.remove("editing"); editor.replaceWith(body); };
      const done = () => { if (!closed) input.value.trim() === typed ? cancel() : (closed = true, saveMarker(m, input.value.trim(), false)); };
      input.addEventListener("keydown", (e) => {
        if (e.key === "Enter") { e.preventDefault(); done(); }
        if (e.key === "Escape") { e.preventDefault(); cancel(); }
      });
      const what = markerLabel(m).toLowerCase();
      const editor = el("div", { class: "marker-edit" }, markerIcon(m), input,
        el("button", { class: "icon-btn save", type: "button", title: "Save (Enter)", "aria-label": "Save", onclick: done }, svg("save", 18)),
        el("button", { class: "icon-btn trash", type: "button", title: `Delete this ${what}`, "aria-label": `Delete this ${what}`,
          onclick: () => { closed = true; saveMarker(m, m.text ?? "", true); } }, svg("trash", 18)));
      // Clicking anywhere else keeps what was typed; Esc leaves it as it was.
      editor.addEventListener("focusout", (e) => { if (!editor.contains(e.relatedTarget)) done(); });
      body.replaceWith(editor);
      line.scrollIntoView({ block: "nearest", behavior: "smooth" });
      input.focus();
    };
    body.addEventListener("click", edit);
    body.addEventListener("keydown", (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); edit(); } });
    markerEditors.set(m, edit);
    return line;
  };

  const lineById = new Map(session.segments.map((s) => [s.id, s]));
  for (const seg of showDoubles ? session.segments : scriptLines) {
    while (markers.length && markers[0].at_s <= seg.start_s) {
      // The same person carrying on after a bookmark or action item isn't named again.
      transcript.append(markerLine(markers.shift()));
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
    const text = el("div", { class: "text", contenteditable: "plaintext-only", spellcheck: "true" }, ...lineWords(seg));
    text.addEventListener("click", (e) => {
      const span = e.target.closest?.(".vfix");
      const fix = span && seg.fixes?.[Number(span.dataset.fix)];
      if (!fix || !getSelection().isCollapsed) return;
      fixMenu(span, fix, async () => { if (await save("toggle_fix", { segment: seg.id, fix: Number(span.dataset.fix) })) reload(); });
    });
    text.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); text.blur(); }
      if (e.key === "Escape") { text.replaceChildren(...lineWords(seg)); text.blur(); }
    });
    text.addEventListener("blur", async () => {
      const value = text.textContent.trim();
      if (value !== seg.text && (await save("update_segment", { segment: seg.id, text: value, speaker: null }))) {
        seg.text = value;
        seg.fixes = [];
        seg.edited = true;
        if (!who.querySelector(".edited")) who.append(el("span", { class: "edited" }, "edited"));
      }
    });
    const double = seg.echo_of != null;
    const original = double ? lineById.get(seg.echo_of) : null;
    const keep = double ? el("button", { class: "link", type: "button", title: "Put this line back in the script, as something said that the other line doesn't have" }, "Keep in script") : null;
    keep?.addEventListener("click", async () => { if (await save("keep_segment", { segment: seg.id })) reload(); });
    // In a meeting in two languages, which one each line was heard in; choosing the other hears it again.
    const lineLanguage = seg.language || meetingLanguages[0];
    let languageMark = null;
    if (meetingLanguages.length > 1 && !double) {
      languageMark = el("select", { class: "line-language", title: "The language this line is in. Choosing the other transcribes it again.", "aria-label": "Language of this line" },
        meetingLanguages.map((c) => el("option", { value: c, selected: c === lineLanguage }, languageName(c))));
      languageMark.addEventListener("change", async () => {
        languageMark.disabled = true;
        if (await save("set_line_language", { segment: seg.id, language: languageMark.value })) reload();
        else { languageMark.disabled = false; languageMark.value = lineLanguage; }
      });
    }
    setLanguage([text], lineLanguage);
    const who = el("div", { class: "who" }, select, languageMark,
      seg.overlap ? el("span", { class: "mark", title: "Someone else was speaking at the same time" }, "talking at once") : null,
      seg.edited ? el("span", { class: "edited" }, "edited") : null,
      double ? el("span", { class: "mark", title: original ? `Heard more clearly on ${trackName(original.stream)}: “${original.text}”` : "" },
        `heard better on ${original ? trackName(original.stream) : "another microphone"}${original ? ` at ${clock(original.start_s)}` : ""}`) : null,
      keep);
    const low = seg.confidence < session.low_confidence;
    const suggested = suggestion?.lines.includes(seg.id);
    // An overlapping line always shows its speaker, since the one before may be the other voice.
    const same = previous === seg.speaker && !seg.overlap && !double;
    const line = el("div", { class: `line${low ? " low" : ""}${same ? " same-speaker" : ""}${seg.overlap ? " overlap" : ""}${suggested ? " suggest" : ""}${double ? " double" : ""}` });
    const time = el("button", { class: "time mono", title: streams.length ? "Play from this line" : "No audio to play" }, clock(seg.start_s));
    time.addEventListener("click", () => {
      if (!player) return;
      player.seek(seg.start_s);
      player.play();
    });
    const action = el("div", { class: "line-action" });
    line.append(time, el("div", {}, who, text), action);
    transcript.append(line);
    lineEls.push({ seg, line, text, action });
    previous = double ? null : seg.speaker;
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
    if (!sel.rangeCount) return;
    const i = lineEls.findIndex((e) => e.text.contains(sel.anchorNode));
    if (i < 0) return;
    if (!sel.isCollapsed) {
      // A word or two picked out of a line can be taught to the vocabulary.
      const heard = sel.toString().trim().replace(/^[^\p{L}\p{N}]+|[^\p{L}\p{N}]+$/gu, "");
      const words = heard.split(/\s+/).filter(Boolean).length;
      if (lineEls[i].text.contains(sel.focusNode) && heard && words <= 4) {
        lineEls[i].action.append(actionButton("SPELL", `Teach Voice to spell “${heard}” another way`, false,
          () => showTermEditor(null, { id, heard })));
      }
      return;
    }
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
  // Removing tracks leaves the transcript as it was until it's transcribed again, so several can go first.
  const removed = meta.tracks_removed ?? [];
  const changed = (meta.tracks_changed ?? []).filter((n) => !removed.includes(n));
  if ((removed.length || changed.length) && !meta.compacted) {
    const list = (names) => names.length === 1 ? names[0] : `${names.slice(0, -1).join(", ")} and ${names.at(-1)}`;
    const again = el("button", { class: "primary pill", type: "button", disabled: session.busy || !session.model_ready }, "Transcribe again");
    again.addEventListener("click", () => processAgain(id));
    const said = [
      removed.length ? `Removed ${list(removed)}.` : "",
      changed.length ? `Changed ${list(changed)}: moves play where you put them now.` : "",
      "The transcript catches up when you transcribe the meeting again.",
    ].filter(Boolean).join(" ");
    tracks.append(el("div", { class: "inline-form track-removed", role: "status" }, el("p", {}, said), again));
  }
  if (streams.length) {
    const head = el("span", { class: "playhead" });
    const timeLabel = el("span", { class: "mono hint" });
    const playButton = el("button", { class: "play", "aria-label": "Play" }, svg("play", 18));
    const lanes = el("div", { class: "track-lanes", title: "Click to jump to a moment" });
    // Where each track is quiet in the transcript mix: greyed out there, and not one of the script's own clips.
    const clipSpans = (s) => (s.parts?.length ? s.parts : [[s.start_s, s.end_s]]);
    const quietIn = new Map(streams.map((stream) => [stream, {
      greyed: [
        ...scriptLines.filter((s) => s.stream !== stream && s.also_on?.includes(stream)).map((s) => [s.start_s, s.end_s]),
        ...doubles.filter((s) => s.stream === stream).flatMap(clipSpans),
      ],
      used: scriptLines.filter((s) => s.stream === stream).flatMap(clipSpans),
    }]));
    const within = (spans, t) => spans.some(([a, b]) => t >= a && t < b);
    // A track moved since it was transcribed: its clips are that much later until it's transcribed again.
    const shiftOf = (stream) => session.pending_offsets?.[stream] ?? 0;
    const quietAt = (stream, t) => {
      const own = t - shiftOf(stream);
      return within(quietIn.get(stream).greyed, own) && !within(quietIn.get(stream).used, own);
    };
    const muteButtons = new Map();
    const manualMuted = new Set();
    const applyMix = (t) => {
      for (const stream of streams) {
        const muted = playMix === "transcript" ? quietAt(stream, t) : manualMuted.has(stream);
        const audio = player?.audios.get(stream);
        if (audio && audio.muted !== muted) audio.muted = muted;
        const button = muteButtons.get(stream);
        button.classList.toggle("on", muted);
        button.classList.toggle("auto", playMix === "transcript");
        button.setAttribute("aria-pressed", String(muted));
      }
    };
    const mixLabel = el("span", { class: "mix-switch", role: "group", "aria-label": "What you hear" });
    const drawMix = () => {
      const option = (value, label, title) => {
        const b = el("button", { class: `pill${playMix === value ? " on" : ""}`, type: "button", "aria-pressed": String(playMix === value), title }, label);
        b.addEventListener("click", () => { playMix = value; drawMix(); applyMix(player?.t ?? 0); });
        return b;
      };
      mixLabel.replaceChildren(
        option("transcript", ["Transcript", el("span", { class: "wide-only" }, " mix")], "Hear what the transcript used: each track is muted where its sound is greyed out"),
        option("manual", "Manual", "Every track plays; mute them yourself with M"));
    };
    drawMix();
    for (const stream of streams) {
      const speaker = session.speakers.find((s) => s.key === stream);
      const name = speaker?.name ?? streamName(stream, session.meta.inputs?.length ? session.meta.inputs : prefs.inputs);
      const mute = el("button", { class: "mute", "aria-pressed": "false", title: `Mute ${name}. In the transcript mix it lights up while the track is muted; pressing it switches to Manual.` }, "M");
      mute.addEventListener("click", (e) => {
        e.stopPropagation();
        if (e.detail) mute.blur();
        // Taking over from the transcript mix starts from every track playing.
        if (playMix === "transcript") { playMix = "manual"; manualMuted.clear(); drawMix(); }
        manualMuted.has(stream) ? manualMuted.delete(stream) : manualMuted.add(stream);
        applyMix(player?.t ?? 0);
      });
      muteButtons.set(stream, mute);
      const clip = (start, end, style, title, cls = "") => el("span", {
        class: cls,
        style: `left:${(100 * start) / duration}%;width:${Math.max(0.3, (100 * (end - start)) / duration)}%;${style}`,
        title,
      });
      const spans = (s) => (s.parts?.length ? s.parts : [[s.start_s, s.end_s]]);
      // Greyed first: what this microphone heard that the script takes from another one.
      const greyed = [
        ...scriptLines.filter((s) => s.stream !== stream && s.also_on?.includes(stream))
          .map((s) => clip(s.start_s, s.end_s, "", `Also heard here · the script uses ${trackName(s.stream)} · ${clock(s.start_s)}`, "echo")),
        ...doubles.filter((s) => s.stream === stream).flatMap((s) => {
          const original = lineById.get(s.echo_of);
          return spans(s).map(([a, b]) => clip(a, b, "", `Left out of the script: heard more clearly ${original ? `on ${trackName(original.stream)}` : "elsewhere"} · “${s.text}”`, "echo"));
        }),
      ];
      const used = scriptLines.filter((s) => s.stream === stream).flatMap((s) => spans(s).map(([a, b]) => clip(a, b,
        `background:${strong.get(s.speaker) ?? "var(--key-light)"}`,
        `${session.speakers.find((sp) => sp.key === s.speaker)?.name ?? ""} · ${clock(s.start_s)}`)));
      const imported = session.imports?.find((i) => i.name === stream);
      const theirTicks = (imported?.markers ?? []).filter((m) => !m.removed).map((m) => {
        // Drawn in the strip, which is already moved by any move not yet transcribed.
        const at = imported.offset_s - shiftOf(stream) + m.at_s * (imported.rate || 1);
        const note = markerNote(m);
        const tick = el("i", { class: `theirs${isAction(m) ? " action" : ""}`, style: `left:${(100 * at) / duration}%`,
          title: `${markerLabel(m)} made in this recording · ${clock(at)}${note ? ` · ${note}` : ""} · click to go there` });
        tick.addEventListener("click", (e) => { e.stopPropagation(); player?.seek(at + shiftOf(stream)); });
        return tick;
      });
      const bar = el("span", { class: "bar" }, el("span", { class: "bar-inner", style: `--shift:${shiftOf(stream) / duration}` }, greyed, used, imported ? theirTicks :
        allMarkers.map((m) => {
          const note = markerNote(m);
          // The meeting's own marks stay put when a track's strip is moved.
          const tick = el("i", { class: isAction(m) ? "action" : "", style: `left:${(100 * (m.at_s - shiftOf(stream))) / duration}%`, title: `${markerLabel(m)} · ${clock(m.at_s)}${note ? ` · ${note}` : ""} · click to change or delete` });
          tick.addEventListener("click", (e) => {
            e.stopPropagation();
            player?.seek(m.at_s);
            markerEditors.get(m)?.();
          });
          return tick;
        })));
      // The name opens the track's settings; an imported track also has a menu to sync it or take it out.
      const details = imported ? `${name} · imported · ${placedText(imported, meta.created)}` : trackDetails(name, meta.tracks?.[stream]);
      const flags = [(meta.locked ?? []).includes(stream) ? "everything on it is them" : "", meta.track_offsets?.[stream] ? `moved ${signedClock(meta.track_offsets[stream])}` : ""].filter(Boolean);
      const nameEl = el("span", { class: "name", role: "button", tabindex: "0", style: `color:${strong.get(stream) ?? "inherit"}`,
        title: `${details}${flags.length ? `
${flags.join(" · ")}` : ""}
Click for who is on it and where it starts` }, name);
      const openSettings = (e) => { e.stopPropagation(); if (!meta.compacted) trackDialog(stream, name); };
      nameEl.addEventListener("click", openSettings);
      nameEl.addEventListener("keydown", (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); openSettings(e); } });
      // A recorded track can be taken out too, as one that heard nothing or doubles another.
      const menu = imported ? importMenu(id, imported, name, page, meta.created)
        : streams.length > 1 && !meta.compacted && !session.busy ? recordedTrackMenu(stream, name) : null;
      lanes.append(el("div", { class: "track" }, menu ? el("span", { class: "name-wrap" }, nameEl, menu) : nameEl, mute, bar));
    }
    lanes.append(head);
    // Zoom: the part of the meeting the bars show, from `pan` (a fraction of it) at `zoom` times. The wheel
    // zooms around the pointer (Shift or a sideways scroll moves along); two fingers pinch, one drags.
    if (trackView.id !== id) trackView = { id, zoom: 1, pan: 0 };
    const maxZoom = Math.max(1, duration / 4);
    const fitButton = el("button", { class: "pill zoom-fit", type: "button", title: "Show the whole meeting again (zoom with the mouse wheel; pinch on a touch screen)" });
    const barRect = () => lanes.querySelector(".bar").getBoundingClientRect();
    const setView = (zoom, pan) => {
      zoom = Math.min(maxZoom, Math.max(1, zoom));
      pan = Math.min(1 - 1 / zoom, Math.max(0, pan));
      const same = Math.abs(zoom - trackView.zoom) < 1e-6 && Math.abs(pan - trackView.pan) < 1e-9;
      trackView.zoom = zoom;
      trackView.pan = pan;
      lanes.style.setProperty("--zoom", zoom);
      lanes.style.setProperty("--pan", pan);
      lanes.classList.toggle("zoomed", zoom > 1.001);
      fitButton.hidden = zoom <= 1.001;
      fitButton.textContent = `${zoom < 10 ? zoom.toFixed(1) : Math.round(zoom)}× · Fit`;
      if (player) onTick(player.t, player.playing);
      return !same;
    };
    // The meeting's time at a point across the bars, as they're zoomed.
    const timeAt = (clientX) => {
      const rect = barRect();
      return (trackView.pan + (clientX - rect.left) / rect.width / trackView.zoom) * duration;
    };
    const zoomAround = (clientX, zoom) => {
      const rect = barRect();
      const fx = Math.min(1, Math.max(0, (clientX - rect.left) / rect.width));
      const at = trackView.pan + fx / trackView.zoom;
      const next = Math.min(maxZoom, Math.max(1, zoom));
      return setView(next, at - fx / next);
    };
    fitButton.addEventListener("click", () => setView(1, 0));
    let dragged = false;
    lanes.addEventListener("click", (e) => {
      if (dragged) { dragged = false; return; }
      const rect = barRect();
      if (e.clientX >= rect.left && e.clientX <= rect.right) player?.seek(timeAt(e.clientX));
    });
    lanes.addEventListener("wheel", (e) => {
      const rect = barRect();
      if (e.clientX < rect.left || e.clientX > rect.right) return;
      const sideways = e.shiftKey || Math.abs(e.deltaX) > Math.abs(e.deltaY);
      const delta = (sideways ? (e.deltaX || e.deltaY) : e.deltaY) * (e.deltaMode === 1 ? 33 : 1);
      const changed = sideways
        ? setView(trackView.zoom, trackView.pan + delta / rect.width / trackView.zoom)
        : zoomAround(e.clientX, trackView.zoom * Math.exp(-delta * 0.002));
      if (changed) e.preventDefault();
    }, { passive: false });
    // Holding the mouse down and dragging moves along a zoomed timeline; a click without moving still seeks.
    let mouse = null;
    lanes.addEventListener("pointerdown", (e) => {
      if (e.pointerType !== "mouse" || e.button !== 0 || trackView.zoom <= 1.001 || e.target.closest("button, [role=button], i")) return;
      mouse = { x: e.clientX, pan: trackView.pan, moved: false, id: e.pointerId };
    });
    lanes.addEventListener("pointermove", (e) => {
      if (!mouse || e.pointerId !== mouse.id) return;
      const dx = e.clientX - mouse.x;
      if (!mouse.moved && Math.abs(dx) > 4) {
        mouse.moved = true;
        lanes.setPointerCapture(e.pointerId);
        lanes.classList.add("dragging");
      }
      if (mouse.moved) setView(trackView.zoom, mouse.pan - dx / barRect().width / trackView.zoom);
    });
    const endDrag = () => {
      if (mouse?.moved) {
        dragged = true;
        setTimeout(() => { dragged = false; }, 0);
      }
      mouse = null;
      lanes.classList.remove("dragging");
    };
    lanes.addEventListener("pointerup", endDrag);
    lanes.addEventListener("pointercancel", endDrag);
    let touch = null;
    const spread = (ts) => Math.hypot(ts[0].clientX - ts[1].clientX, ts[0].clientY - ts[1].clientY);
    const middle = (ts) => (ts[0].clientX + ts[1].clientX) / 2;
    lanes.addEventListener("touchstart", (e) => {
      const ts = e.touches;
      if (ts.length === 2) {
        const rect = barRect();
        const fx = (middle(ts) - rect.left) / rect.width;
        touch = { pinch: true, spread: spread(ts), zoom: trackView.zoom, at: trackView.pan + fx / trackView.zoom };
      } else if (ts.length === 1) {
        touch = { pinch: false, x: ts[0].clientX, pan: trackView.pan, moved: false };
      }
    }, { passive: true });
    lanes.addEventListener("touchmove", (e) => {
      if (!touch) return;
      const ts = e.touches;
      const rect = barRect();
      if (touch.pinch && ts.length === 2) {
        e.preventDefault();
        const zoom = Math.min(maxZoom, Math.max(1, touch.zoom * spread(ts) / Math.max(1, touch.spread)));
        setView(zoom, touch.at - (middle(ts) - rect.left) / rect.width / zoom);
      } else if (!touch.pinch && ts.length === 1 && trackView.zoom > 1.001) {
        const dx = ts[0].clientX - touch.x;
        if (Math.abs(dx) > 8) touch.moved = true;
        if (touch.moved) {
          e.preventDefault();
          setView(trackView.zoom, touch.pan - dx / rect.width / trackView.zoom);
        }
      }
    }, { passive: false });
    lanes.addEventListener("touchend", (e) => {
      if (touch?.moved || touch?.pinch) {
        dragged = true;
        setTimeout(() => { dragged = false; }, 400);
      }
      if (!e.touches.length) touch = null;
    });
    const onTick = (t, playing) => {
      const bar = lanes.querySelector(".bar");
      if (bar) {
        // While playing, the view follows the playhead.
        const { zoom, pan } = trackView;
        const at = t / duration;
        if (playing && zoom > 1.001 && (at < pan || at > pan + 1 / zoom)) {
          trackView.pan = Math.min(1 - 1 / zoom, Math.max(0, at - 0.1 / zoom));
          lanes.style.setProperty("--pan", trackView.pan);
        }
        const lanesRect = lanes.getBoundingClientRect();
        const rect = bar.getBoundingClientRect();
        const fx = (at - trackView.pan) * trackView.zoom;
        head.hidden = fx < 0 || fx > 1;
        head.style.left = `${rect.left - lanesRect.left + rect.width * fx}px`;
      }
      timeLabel.textContent = `${clock(t)} / ${clock(duration)}`;
      if (meta.created) timeLabel.title = `${dayClock(meta.created + t)} by the time of day`;
      // Only when it changes: swapping the icon on every tick removes the element a click pressed
      // down on, and WebView2 then drops the click, so Pause wouldn't work while playing.
      if (playButton.dataset.playing !== String(playing)) {
        playButton.dataset.playing = String(playing);
        playButton.replaceChildren(svg(playing ? "pause" : "play", 18));
        playButton.setAttribute("aria-label", playing ? "Pause" : "Play");
      }
      for (const { seg, line } of lineEls) line.classList.toggle("playing", playing && t >= seg.start_s && t < seg.end_s);
      applyMix(t);
      // Playing from a line of the transcript opens the tracks too.
      if (playing && tracksOpenFor !== id) setTracksOpen(true);
    };
    // On a phone the tracks are a play button and the recording's length until something is played.
    const setTracksOpen = (open) => {
      tracksOpenFor = open ? id : null;
      tracks.classList.toggle("open", open);
      if (open) requestAnimationFrame(() => player && onTick(player.t, player.playing));
    };
    tracks.classList.toggle("open", tracksOpenFor === id);
    const mini = el("span", { class: "tracks-mini" }, el("strong", {}, "Recording"), el("span", { class: "mono hint" }, clock(duration)));
    const fold = el("button", { class: "icon-only tracks-fold", type: "button", title: "Hide the tracks", "aria-label": "Hide the tracks",
      onclick: () => { player?.pause(); setTracksOpen(false); } }, svg("down", 16));
    player = makePlayer(session, duration, onTick);
    playButton.addEventListener("click", () => { if (tracksOpenFor !== id) setTracksOpen(true); player?.toggle(); });
    // Clip starts, bookmarks and action items, for jumping to the previous or next in the timeline.
    // Every clip the transcript mix plays, on every track (each clip of a stitched line too), and the marks.
    const starts = [...scriptLines.flatMap((s) => clipSpans(s).map(([a]) => a)), ...allMarkers.map((m) => m.at_s)]
      .sort((a, b) => a - b)
      .filter((t, i, all) => i === 0 || t - all[i - 1] > 0.05);
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
      el("button", { class: "pill", "aria-label": "Previous clip or mark", title: "Start of the previous clip, bookmark or action item (←)", onclick: prevClip }, svg("skip-back", 16)),
      el("button", { class: "pill", "aria-label": "Back 5 seconds", title: "Back 5 seconds (Alt + ←)", onclick: () => player?.seek(player.t - 5) }, "−5"),
      playButton, mini,
      el("button", { class: "pill", "aria-label": "Forward 5 seconds", title: "Forward 5 seconds (Alt + →)", onclick: () => player?.seek(player.t + 5) }, "+5"),
      el("button", { class: "pill", "aria-label": "Next clip or mark", title: "Start of the next clip, bookmark or action item (→)", onclick: nextClip }, svg("skip-forward", 16)),
      timeLabel, fitButton, el("span", { class: "grow" }), mixLabel,
      meta.compacted ? null : el("button", { class: "pill with-icon", title: "Add another recording of this meeting, such as from a phone, as a track", onclick: () => showImport(tracks, id, meta.created), "aria-label": "Import a recording" }, svg("import", 14), el("span", { class: "wide-only" }, "Import")), fold));
    player.prevClip = prevClip;
    player.nextClip = nextClip;
    setView(trackView.zoom, trackView.pan);
    requestAnimationFrame(() => player?.seek(keepTime));
  }
  // At the end of the transcript, as at the end of a note: copies it all, and says how many words that is.
  const spoken = scriptLines.filter((l) => hasWords(l.text));
  if (spoken.length) {
    const transcriptText = () => spoken.map((l) => `${session.speakers.find((sp) => sp.key === l.speaker)?.name ?? "Someone"}: ${l.text}`).join("\n");
    page.append(el("div", { class: "note-end" }, copyIconButton(transcriptText, "Copy transcript"),
      el("span", { class: "hint" }, wordsText(wordCount(spoken.map((l) => l.text).join(" "))))));
  }
  setLanguage([page], meetingLanguage());
  pinTopBar(topBar);
  main.replaceChildren(topBar, el("div", { class: "review" }, page, streams.length ? tracks : null));
}

// ---------- Importing tracks

/// Seconds as "1:02.5", with a minus sign before the meeting started.
function signedClock(seconds) {
  const sign = seconds < 0 ? "−" : "";
  const s = Math.abs(seconds);
  return `${sign}${Math.floor(s / 60)}:${(s % 60).toFixed(1).padStart(4, "0")}`;
}

/// "12.5", "1:02.5", "-0:05" or "−5" as seconds; null when it isn't a time.
function parseStart(text) {
  const t = text.trim().replace("−", "-");
  const m = t.match(/^(-)?(?:(\d+):)?(\d+(?:\.\d+)?)$/);
  if (!m) return null;
  const seconds = Number(m[2] ?? 0) * 60 + Number(m[3]);
  return m[1] ? -seconds : seconds;
}

const syncQuality = (c) => (c >= 1.5 ? "a clear match" : c >= 1.15 ? "a fair match" : "a weak match: check it");

/// Decodes any audio the web view can play into 16 kHz mono 16-bit samples.
async function decodeTo16k(file) {
  const context = new AudioContext();
  let audio;
  try {
    audio = await context.decodeAudioData(await file.arrayBuffer());
  } finally {
    context.close().catch(() => {});
  }
  const offline = new OfflineAudioContext(1, Math.ceil(audio.duration * 16000), 16000);
  const source = offline.createBufferSource();
  source.buffer = audio;
  source.connect(offline.destination);
  source.start();
  const out = (await offline.startRendering()).getChannelData(0);
  const pcm = new Int16Array(out.length);
  for (let i = 0; i < out.length; i++) pcm[i] = Math.max(-1, Math.min(1, out[i])) * 32767;
  return new Uint8Array(pcm.buffer);
}

/// "14:30:05" for a time of day in Unix seconds.
function dayClock(unix) {
  return new Date(unix * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false });
}

/// A time of day typed as "14:30", "14:30:05" or "2:30:05 pm", on the meeting's day (or the day either side,
/// whichever is closest to the meeting), as Unix seconds; null when it isn't a time.
function parseDayClock(text, meetingStart) {
  const m = text.trim().toLowerCase().match(/^(\d{1,2})[:.](\d{2})(?:[:.](\d{2}(?:\.\d+)?))?\s*(am|pm)?$/);
  if (!m) return null;
  let hours = Number(m[1]);
  if (m[4] === "pm" && hours < 12) hours += 12;
  if (m[4] === "am" && hours === 12) hours = 0;
  if (hours > 23 || Number(m[2]) > 59) return null;
  const day = new Date(meetingStart * 1000);
  day.setHours(hours, Number(m[2]), 0, 0);
  const at = day.getTime() / 1000 + Number(m[3] ?? 0);
  return [at - 86400, at, at + 86400].reduce((a, b) => (Math.abs(b - meetingStart) < Math.abs(a - meetingStart) ? b : a));
}

/// When a recording started, from its file: a date and time in its name, as phones and recorders write them
/// ("20261111_143005", "2026-11-11 14.30.05"), or else when the file was last changed less its length.
/// { at, from } or null.
function recordingStart(file, seconds) {
  const m = file.name.match(/(20\d\d)[-_.]?(\d\d)[-_.]?(\d\d)[ T_-]?(\d\d)[-_.:h]?(\d\d)[-_.:m]?(\d\d)/);
  if (m) {
    const at = new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]), Number(m[4]), Number(m[5]), Number(m[6])).getTime() / 1000;
    if (Number.isFinite(at)) return { at, from: "the file's name" };
  }
  if (file.lastModified && seconds) return { at: file.lastModified / 1000 - seconds, from: "when the file was saved, less its length" };
  return null;
}

/// How long a recording lasts, from its file, without decoding it all.
function audioSeconds(file) {
  return new Promise((resolve) => {
    const audio = new Audio();
    const url = URL.createObjectURL(file);
    const done = (v) => { URL.revokeObjectURL(url); resolve(v); };
    audio.preload = "metadata";
    audio.onloadedmetadata = () => done(Number.isFinite(audio.duration) ? audio.duration : null);
    audio.onerror = () => done(null);
    setTimeout(() => done(null), 4000);
    audio.src = url;
  });
}

/// How an imported track was placed, for its title and menu.
function placedText(imported, meetingStart) {
  const how = imported.placed_by === "sound" ? `matched by its sound (${syncQuality(imported.confidence ?? 0)})`
    : imported.placed_by === "hand" ? "placed by hand" : "placed by the time of day";
  const drift = imported.rate && imported.rate !== 1 ? ` · its clock ran ${Math.abs((imported.rate - 1) * 100).toFixed(2)} % ${imported.rate > 1 ? "slow" : "fast"}` : "";
  return `Starts ${dayClock(meetingStart + imported.offset_s)}, ${signedClock(imported.offset_s)} into the meeting · ${how}${drift}`;
}

/// The form for adding a recording as a track: the file, its name, and where it goes. The time of day it
/// started places it first, read from the file and changeable; matching its sound can then fine-tune it.
function showImport(tracks, id, meetingStart) {
  tracks.querySelector(".import-form")?.remove();
  const file = el("input", { type: "file", accept: "audio/*,.wav,.mp3,.m4a,.aac,.ogg,.flac,.webm", "aria-label": "Audio file" });
  const name = el("input", { type: "text", placeholder: "Track name, such as Phone", maxlength: "60", "aria-label": "Track name" });
  const how = el("select", { "aria-label": "How to place it" },
    el("option", { value: "time" }, "By the time of day it started"),
    el("option", { value: "sound" }, "By matching its sound anywhere"),
    el("option", { value: "at" }, "At a point in the meeting"));
  const when = el("input", { type: "text", class: "mono", size: "10", value: dayClock(meetingStart), "aria-label": "Started at, time of day" });
  const whenFrom = el("small", { class: "hint" }, `The meeting started at ${dayClock(meetingStart)}.`);
  const refine = el("input", { type: "checkbox", checked: true });
  const at = el("input", { type: "text", value: "0:00.0", class: "mono", size: "8", "aria-label": "Starts at" });
  const status = el("div", { class: "hint", role: "status" });
  const timeRows = [
    el("label", { class: "import-row" }, el("span", {}, "It started at"), when, whenFrom),
    el("label", { class: "import-row check" }, refine, el("span", {}, "Then fine-tune it by matching its sound nearby",
      el("small", { class: "hint" }, " Takes up a call's delay and clocks that run apart. Leave it off for two sides of a call that share no sound."))),
  ];
  const atRow = el("label", { class: "import-row" }, el("span", {}, "It starts at"), at, el("small", { class: "hint" }, "minutes:seconds into the meeting; −0:05 if it started first"));
  const showRows = () => {
    timeRows.forEach((r) => { r.hidden = how.value !== "time"; });
    atRow.hidden = how.value !== "at";
  };
  how.addEventListener("change", showRows);
  file.addEventListener("change", async () => {
    const chosen = file.files[0];
    if (!chosen) return;
    if (!name.value) name.value = chosen.name.replace(/\.[^.]+$/, "");
    const found = recordingStart(chosen, await audioSeconds(chosen));
    if (!found) return;
    when.value = dayClock(found.at);
    const apart = Math.abs(found.at - meetingStart);
    whenFrom.textContent = `From ${found.from}. The meeting started at ${dayClock(meetingStart)}.${apart > 6 * 3600 ? " That's hours apart: check it." : ""}`;
  });
  const go = el("button", { class: "primary pill" }, "Import and transcribe");
  go.addEventListener("click", async () => {
    const chosen = file.files[0];
    if (!chosen) return status.replaceChildren(el("span", { class: "danger" }, "Choose a recording first."));
    const headers = { "x-id": id, "x-label": encodeURIComponent(name.value.trim() || chosen.name), "x-place": how.value };
    if (how.value === "time") {
      const started = parseDayClock(when.value, meetingStart);
      if (started == null) return status.replaceChildren(el("span", { class: "danger" }, "Type the time it started as hours:minutes:seconds, such as 14:30:05."));
      headers["x-started-at"] = String(started);
      headers["x-refine"] = refine.checked ? "1" : "0";
    } else if (how.value === "at") {
      const offset = parseStart(at.value);
      if (offset == null) return status.replaceChildren(el("span", { class: "danger" }, "Type the start as minutes:seconds, such as 1:02.5, or −0:05 if it started first."));
      headers["x-offset"] = String(offset);
    }
    go.disabled = true;
    try {
      status.replaceChildren(el("span", { class: "spinner" }), " Reading the recording…");
      const pcm = await decodeTo16k(chosen);
      status.replaceChildren(el("span", { class: "spinner" }), how.value === "at" || (how.value === "time" && !refine.checked) ? " Adding it…" : " Matching its sound to the other tracks…");
      const added = await invoke("import_track", pcm, { headers });
      const missed = how.value === "time" && refine.checked && added.placed_by !== "sound" ? " Its sound didn't clearly match nearby, so it's placed by the time of day." : "";
      status.replaceChildren(`${placedText(added, meetingStart)}.${missed} Transcribing…`);
      processAgain(id);
    } catch (e) {
      go.disabled = false;
      status.replaceChildren(el("span", { class: "danger" }, `Couldn't import it: ${e?.message ?? e}`));
    }
  });
  // Someone else's Superteem Voice session of the same meeting: every track of theirs that heard something,
  // placed by when their recording started (the same instant wherever they were), then by its sound.
  const owner = el("input", { type: "text", placeholder: "Whose, such as Lin", maxlength: "40", "aria-label": "Whose recording" });
  const sessionStatus = el("div", { class: "hint", role: "status" });
  const pickSession = (zip) => async () => {
    const picked = await window.__TAURI__.dialog.open(zip
      ? { title: "Their zipped session", filters: [{ name: "Zipped session", extensions: ["zip"] }] }
      : { directory: true, title: "Their session folder (session-…)" }).catch(() => null);
    if (!picked) return;
    sessionButtons.forEach((b) => { b.disabled = true; });
    sessionStatus.replaceChildren(el("span", { class: "spinner" }), " Reading their tracks and matching them to yours…");
    try {
      const done = await invoke("import_session", { id, path: picked, owner: owner.value.trim() });
      const silent = done.silent ? ` ${done.silent} that heard nothing ${done.silent === 1 ? "was" : "were"} left out.` : "";
      const placed = done.matched ? placedText(done.added[0], meetingStart)
        : `Their recording started at ${dayClock(done.started_at)}, ${signedClock(done.started_at - meetingStart)} into the meeting · its sound didn't clearly match nearby, so it's placed by the time of day`;
      sessionStatus.replaceChildren(`Added ${done.added.length} track${done.added.length === 1 ? "" : "s"}. ${placed}.${silent} Transcribing…`);
      processAgain(id);
    } catch (e) {
      sessionButtons.forEach((b) => { b.disabled = false; });
      sessionStatus.replaceChildren(el("span", { class: "danger" }, `Couldn't import it: ${e?.message ?? e}`));
    }
  };
  const sessionButtons = [
    el("button", { class: "pill", type: "button", onclick: pickSession(false) }, "Choose their session folder…"),
    el("button", { class: "pill", type: "button", onclick: pickSession(true) }, "Choose a .zip…"),
  ];
  const fromSession = isDesktop() ? [
    el("div", { class: "menu-label" }, "Someone else's Superteem Voice recording"),
    el("label", { class: "import-row" }, el("span", {}, "Whose"), owner, el("small", { class: "hint" }, "Put in front of their track names")),
    el("div", { class: "import-row" }, ...sessionButtons),
    sessionStatus,
    el("div", { class: "menu-label" }, "A recording from anything else"),
  ] : [];
  const form = el("div", { class: "import-form" },
    el("div", { class: "import-head" }, el("strong", {}, "Import a recording"),
      el("button", { class: "icon-only", "aria-label": "Close", onclick: () => form.remove() }, svg("close", 14))),
    el("p", { class: "hint" }, "Another recording of this meeting becomes a track: a phone in the room, or the other side of a call recorded by the person on it. It's transcribed with the others, so the transcript has everyone."),
    ...fromSession,
    el("label", { class: "import-row" }, el("span", {}, "Recording"), file),
    el("label", { class: "import-row" }, el("span", {}, "Name"), name),
    el("label", { class: "import-row" }, el("span", {}, "Place it"), how),
    ...timeRows, atRow,
    el("div", { class: "import-row" }, go), status);
  showRows();
  tracks.append(form);
  form.scrollIntoView({ block: "nearest", behavior: "smooth" });
}

/// The menu on an imported track's name.
function importMenu(id, imported, label, page, meetingStart) {
  const fail = (e) => showError(page, e);
  const inline = (text, input, save) => {
    const button = el("button", { class: "primary pill" }, "Save and transcribe");
    const box = el("div", { class: "inline-form" }, `${label} ${text}`, input, button,
      el("button", { class: "pill", onclick: () => box.remove() }, "Cancel"));
    button.addEventListener("click", () => save(box));
    page.querySelector("h1")?.before(box);
    input.focus();
  };
  const match = (near) => async () => {
    try {
      await invoke("sync_track", { id, name: imported.name, near });
      processAgain(id);
    } catch (e) { fail(e); }
  };
  return menuButton(svg("more", 14), [
    "Where it goes",
    ["Fine-tune by its sound", "Matches it to the other tracks near where it is, and finds clocks that ran apart", match(true)],
    ["Match its sound anywhere", "When the time of day is far out", match(false)],
    ["Set the time of day it started", dayClock(meetingStart + imported.offset_s), () => {
      const input = el("input", { type: "text", class: "mono", size: "10", value: dayClock(meetingStart + imported.offset_s), "aria-label": "Started at, time of day" });
      inline("started at", input, async (box) => {
        const started = parseDayClock(input.value, meetingStart);
        if (started == null) return showError(box, "Type hours:minutes:seconds, such as 14:30:05.");
        try { await invoke("set_track_start", { id, name: imported.name, startedAt: started }); processAgain(id); } catch (e) { showError(box, e); }
      });
    }],
    ["Set where it starts in the meeting", `${signedClock(imported.offset_s)} now`, () => {
      const input = el("input", { type: "text", class: "mono", size: "8", value: signedClock(imported.offset_s).replace("−", "-"), "aria-label": "Starts at" });
      inline("starts at", input, async (box) => {
        const offset = parseStart(input.value);
        if (offset == null) return showError(box, "Type minutes:seconds, such as 1:02.5.");
        try { await invoke("set_track_offset", { id, name: imported.name, offsetS: offset }); processAgain(id); } catch (e) { showError(box, e); }
      });
    }],
    "This track",
    ["Remove this track", "Its lines leave the transcript", async () => {
      try { await invoke("remove_track", { id, name: imported.name }); processAgain(id); } catch (e) { fail(e); }
    }],
  ]);
}

/// A phrase heard again this close to a marker made while recording is the same moment.
const SAME_MARK_S = 5;
const PHRASE_KINDS = { bookmark: "", action: "action" };

/// The markers made while recording, and "Bookmark that" or "Action item" heard in the recording when it was
/// processed, as when nothing was listening for them at the time.
/// Saved markers keep their place in the list, for changing them; deleted ones still hide the phrase heard there.
function sessionMarkers(saved, phrases) {
  const all = saved.map((m, index) => ({ ...m, index }));
  const heard = phrases
    .filter((h) => !h.rejected && h.phrase in PHRASE_KINDS)
    .map((h) => ({ at_s: Math.round(h.start_s * 10) / 10, text: "", kind: PHRASE_KINDS[h.phrase], heard: true }))
    .filter((h) => !all.some((m) => isAction(m) === isAction(h) && Math.abs(m.at_s - h.at_s) < SAME_MARK_S));
  return [...all.filter((m) => !m.removed), ...heard].sort((a, b) => a.at_s - b.at_s);
}

/// What a marker says: the note typed with it, or suggested for it. An action item has none until then.
function markerNote(m) {
  return m.text && m.text !== "Action item" ? m.text : "";
}

/// Action items having a label suggested, as "meeting id/seconds".
const labelling = new Set();
/// The meeting page's tracks are folded to a play button on a phone until something is played: the meeting they're open for.
let tracksOpenFor = null;
/// How far the tracks are zoomed in, kept while a meeting is redrawn.
let trackView = { id: null, zoom: 1, pan: 0 };

// A suggested label for an action item has been written, or couldn't be.
listen("analysed", async (e) => {
  const { id, template, error } = e.payload;
  if (!template.startsWith("label:")) return;
  labelling.delete(`${id}/${Number(template.slice(6))}`);
  if (view.kind !== "session" || view.id !== id) return;
  const fresh = await invoke("get_session", { id }).catch(() => null);
  if (!fresh || view.kind !== "session" || view.id !== id) return;
  renderSession(fresh, null);
  if (error && error !== "Cancelled.") showError(main.querySelector(".page"), `Couldn't suggest a label: ${error}`);
});

/// The keyboard shortcuts, as shown on the Shortcuts page. Each is [keys, what it does].
const SHORTCUTS = [
  ["Anywhere", [
    [["Ctrl", "N"], "New note"],
    [["Ctrl", "M"], "Start a meeting"],
    [["Alt", "V"], "Start or stop dictation (in a note: dictate there; elsewhere: in a new note)"],
    [["Ctrl", "Space"], "Start or finish dictation in a note"],
    [["Esc"], "Cancel dictation"],
  ]],
  ["Recording a meeting", [
    [["B"], "Bookmark this moment, when not typing"],
    [["A"], "Add an action item, when not typing"],
    [["Alt", "B"], "Bookmark this moment, even while typing a note"],
    [["Alt", "A"], "Add an action item, even while typing a note"],
    [["Enter"], "In the note box: bookmark with the note"],
    [["Ctrl", "Enter"], "In the note box: action item with the note"],
  ]],
  ["Playing back a meeting, when not typing", [
    [["Space"], "Play or pause"],
    [["←", "→"], "Previous or next clip, bookmark or action item"],
    [["Alt", "←", "→"], "Back or forward 5 seconds"],
    [["Shift", "←", "→"], "Back or forward 20 seconds"],
    [["Ctrl", "←", "→"], "Back or forward 1 second"],
  ]],
];

/// Typing somewhere: single keys and arrows belong to the text there.
function typingIn(target) {
  return !!target && (target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName));
}

window.addEventListener("keydown", (e) => {
  if (e.defaultPrevented || e.repeat && !["ArrowLeft", "ArrowRight"].includes(e.key)) return;
  const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
  const t = e.target;
  // Anywhere in the app.
  if (e.ctrlKey && !e.altKey && !e.shiftKey && key === "n") {
    e.preventDefault();
    openNote(null);
    return;
  }
  if (e.ctrlKey && !e.altKey && !e.shiftKey && key === "m") {
    e.preventDefault();
    showMeeting();
    return;
  }
  if (e.altKey && !e.ctrlKey && key === "v" && !isPhone()) {
    e.preventDefault();
    if (view.kind === "note") dictation ? stopDictation() : startDictation();
    else if (!recording) openNote(null, { dictate: true });
    return;
  }
  // Recording a meeting: single keys when not typing, and with Alt always.
  if (view.kind === "recorder" && recording && liveMark && !e.ctrlKey && (key === "b" || key === "a")) {
    if (!e.altKey && typingIn(t)) return;
    e.preventDefault();
    liveMark(key === "a" ? "action" : "");
    return;
  }
  // Playing back a meeting.
  if (view.kind !== "session" || !player || typingIn(t)) return;
  if (e.code === "Space" && !e.ctrlKey && !e.altKey) {
    // A track's mute button keeps Space for playing and pausing; other buttons press as usual.
    if ((t.tagName === "BUTTON" || t.getAttribute?.("role") === "button") && !t.classList?.contains("mute")) return;
    e.preventDefault();
    player.toggle();
  } else if (key === "ArrowLeft" || key === "ArrowRight") {
    const back = key === "ArrowLeft";
    e.preventDefault();
    if (e.altKey) player.seek(player.t + (back ? -5 : 5));
    else if (e.shiftKey) player.seek(player.t + (back ? -20 : 20));
    else if (e.ctrlKey) player.seek(player.t + (back ? -1 : 1));
    else back ? player.prevClip?.() : player.nextClip?.();
  }
});

function showShortcuts() {
  leave();
  showPage();
  view = { kind: "shortcuts" };
  refreshItems();
  const keys = (combo) => el("span", { class: "keys" }, combo.flatMap((k, i) => [i ? el("span", { class: "plus" }, k === "→" ? "/" : "+") : null, el("kbd", {}, k)]));
  main.replaceChildren(el("div", { class: "page shortcuts" }, el("h1", {}, "Keyboard shortcuts"),
    SHORTCUTS.flatMap(([group, rows]) => [el("h2", {}, group), el("div", { class: "group" },
      rows.map(([combo, what]) => el("div", { class: "row" }, el("span", { class: "label" }, what), keys(combo))))])));
}

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
  // The list opens from a plain arrow; the select itself is invisible over it, so no "⋯" shows cut short.
  return el("span", { class: "chip-menu" }, svg("down", 14), menu);
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
    el("button", { class: "link", onclick: () => showVoices() }, "‹ Voices"),
    el("h1", {}, "Voice prints"),
    el("p", { class: "legend" }, "A voice print lets Voice recognise someone and name them in new meetings. "
      + "It is kept only on this device, as a description of how the voice sounds; the recording made for it is deleted. "
      + "Only make one with the person's agreement."));
  if (error) page.append(el("div", { class: "error" }, String(error)));
  if (!prints.length) page.append(el("div", { class: "empty" }, "No voice prints yet."));
  // One person can have several prints, as they sound different on each microphone and through call apps.
  const people = [];
  for (const p of prints) {
    const person = people.find((x) => x.id === p.person);
    if (person) person.prints.push(p); else people.push({ id: p.person, name: p.name, prints: [p] });
  }
  for (const person of people) {
    const first = person.prints[0];
    const name = el("input", { value: person.name, title: "Rename", "aria-label": "Name" });
    name.addEventListener("keydown", (e) => { if (e.key === "Enter") name.blur(); });
    name.addEventListener("change", async () => {
      try { await invoke("rename_voiceprint", { id: first.id, name: name.value }); showVoicePrints(); } catch (e) { showError(page, e); }
    });
    const others = people.filter((x) => x !== person);
    const same = el("select", { class: "plain", "aria-label": `${person.name} is the same person as` },
      el("option", { value: "" }, "Same person as…"), others.map((o) => el("option", { value: o.prints[0].id }, o.name)));
    same.addEventListener("change", async () => {
      if (!same.value) return;
      try { await invoke("merge_voiceprint_people", { id: first.id, into: same.value }); showVoicePrints(); } catch (e) { showError(page, e); }
    });
    page.append(el("div", { class: "print-row person-row" }, name,
      el("span", { class: "detail" }, person.prints.length > 1 ? `${person.prints.length} voice prints` : ""),
      el("button", { onclick: () => showEnrol(null, first) }, "Add a voice print"),
      others.length ? same : null));
    for (const p of person.prints) page.append(printRow(p, person.prints.length > 1));
  }
  page.append(el("div", { class: "actions" }, el("button", { class: "primary pill", onclick: () => showEnrol(null) }, "New voice print")),
    el("p", { class: "legend" }, "You can also make a voice print from a speaker in any meeting, from the ⋯ menu beside their name. "
      + "Someone can have several, as they sound different on each microphone and through call apps; each is matched separately."));
  main.replaceChildren(page);

  /// One voice print of a person: what it was recorded on and from what, with its own buttons.
  function printRow(p, shared) {
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
    const separate = shared ? el("button", { class: "link", title: "This voice print is someone else with the same name" }, "Not them") : null;
    separate?.addEventListener("click", async () => {
      try { await invoke("separate_voiceprint", { id: p.id }); showVoicePrints(); } catch (e) { showError(page, e); }
    });
    return el("div", { class: "print-row print-sub" },
      el("span", { class: "label" }, p.label || "Voice print"),
      el("span", { class: "detail" }, `${Math.round(p.seconds)} s of speech from ${sources.join(" and ")}`),
      el("button", { onclick: () => showEnrol(p) }, "Record more"), separate, remove);
  }
}

/// Sets up a script reading for a new voice print, or more speech for `print`.
/// With `forPerson` (one of their prints), another voice print for someone, as on another microphone.
async function showEnrol(print, forPerson = null) {
  leave();
  showPage();
  view = { kind: "prints" };
  const [{ devices, default: preferred }, fresh] = await Promise.all([invoke("list_devices"), invoke("get_prefs")]);
  prefs = fresh;
  const page = el("div", { class: "page" }, el("h1", {}, print ? `Record more of ${print.name}` : forPerson ? `Another voice print for ${forPerson.name}` : "New voice print"));
  const name = el("input", { type: "text", class: "plain", placeholder: "Their name", "aria-label": "Their name" });
  if (!print && !forPerson) page.append(el("h2", {}, "Name"), name);

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
        showEnrol(print, forPerson);
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
        showEnrol(print, forPerson);
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
      await invoke("start_enrolment", { device: input.value, name: name.value, print: print?.id ?? null, person: forPerson?.id ?? null, consent: consent.checked });
      enrolment = { print, name: print?.name ?? forPerson?.name ?? name.value.trim(), script: scripts[choice.value].text };
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
    el("button", { class: "link", onclick: () => showVoices() }, "‹ Voices"),
    el("h1", {}, "Wake word and commands"),
    el("p", { class: "legend" }, "Say each phrase a few times so Voice hears it the moment it's said, without waiting "
      + "for speech to text. It learns how you say them, so it answers you rather than anyone in the room. Only a "
      + "description of the sound is kept with your voice print; the recordings are deleted. Record again throws "
      + "away every take kept for that phrase and starts it fresh."));
  if (!print) {
    page.append(el("div", { class: "notice" }, "Make your voice print first: the phrases are kept with it."),
      el("div", { class: "actions" }, el("button", { class: "primary pill", onclick: () => showOnboarding(SETUP.voice) }, "Make my voice print")));
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
      ? [el("button", { class: "pill", title: "Replaces all the takes kept now", onclick: () => record(phrase, true) }, "Record again"),
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
      if (h.unconfirmed) {
        return show(h.words ? `Ignored: it sounded a little like “${h.text}”, but the words were “${h.words}”` : `Ignored: it sounded a little like “${h.text}”, but no words were heard`);
      }
      if (h.rejected) return show(`“${h.text}” didn't sound like ${h.who || "you"}, so it would be ignored · ${timing}`);
      if (h.phrase === "wake") {
        chime();
        return show(`“Hey Superteem” · ${timing}. Now say a command.`);
      }
      const command = commandName(KEYPHRASE_COMMANDS[h.phrase]) ?? `the answer “${h.text}”`;
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
      + "differently each time: that teaches Voice the ways you really say it."
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

/// How a term's "When" reads in the list.
function whenText(when) {
  if (when?.kind === "near") return `Only near ${when.words.map((w) => `“${w}”`).join(", ")}`;
  if (when?.kind === "suggest") return "Suggested, not written in";
  return "Always";
}

const splitList = (text) => text.split(",").map((s) => s.trim()).filter(Boolean);

/// The person's vocabulary: words speech to text should spell their way.
async function showVocabulary(message) {
  leave();
  showPage();
  view = { kind: "vocabulary" };
  refreshItems();
  const terms = await invoke("list_vocabulary").catch(() => []);
  const page = el("div", { class: "page" },
    el("button", { class: "link", onclick: () => showTranscription() }, "‹ Transcription"),
    el("h1", {}, "Vocabulary"),
    el("p", { class: "legend" }, "Names, products and acronyms that speech to text writes as everyday words, such as “SIBR” "
      + "written as “cyber”. Voice tells the speech model about these words before it listens, and corrects what it "
      + "still gets wrong. In a meeting, each corrected word is underlined: click it to undo the change."));
  if (message) page.append(el("p", { class: "vocab-notice", role: "status" }, message));
  if (!terms.length) page.append(el("div", { class: "empty" }, "No words yet."));
  for (const t of terms) {
    const remove = el("button", { class: "link danger" }, "Delete");
    remove.addEventListener("click", async () => {
      if (!remove.dataset.armed) {
        remove.dataset.armed = "1";
        remove.textContent = "Click again to delete";
        setTimeout(() => { delete remove.dataset.armed; remove.textContent = "Delete"; }, 4000);
        return;
      }
      try { await invoke("delete_vocab_term", { id: t.id }); showVocabulary(); } catch (e) { showError(page, e); }
    });
    page.append(el("div", { class: "print-row vocab-row" },
      el("span", { class: "vocab-word" }, t.spelling),
      el("span", { class: "detail" }, [
        t.sounds_like.length ? `Sounds like ${t.sounds_like.map((s) => `“${s}”`).join(", ")}` : "No sound-alikes yet",
        t.letters ? "also said letter by letter" : null,
        whenText(t.when),
        t.uses ? `used ${t.uses} time${t.uses === 1 ? "" : "s"}` : null,
      ].filter(Boolean).join(" · ")),
      el("button", { onclick: () => showTermEditor(t) }, "Edit"), remove));
  }
  const fix = el("button", { class: "pill", hidden: true });
  page.append(el("div", { class: "actions" }, el("button", { class: "primary pill", onclick: () => showTermEditor(null) }, "Add a word"), fix));
  main.replaceChildren(page);
  if (!terms.length) return;
  // Meetings made before a word was added still have the everyday word; offer to correct them.
  const count = await invoke("fix_past_transcripts", { dryRun: true, only: null }).catch(() => 0);
  if (!count || !page.isConnected) return;
  fix.hidden = false;
  fix.textContent = `Correct ${count} word${count === 1 ? "" : "s"} in past meetings`;
  fix.title = "Lines you edited by hand are left alone. Each change can be undone from the meeting.";
  fix.addEventListener("click", async () => {
    fix.disabled = true;
    try {
      const done = await invoke("fix_past_transcripts", { dryRun: false, only: null });
      showVocabulary(`Corrected ${done} word${done === 1 ? "" : "s"} in past meetings.`);
    } catch (e) {
      fix.disabled = false;
      showError(page, e);
    }
  });
}

/// Adds or edits a term. `from` is a meeting the person picked the word in, as { id, heard }: saving
/// corrects that meeting and returns to it.
function showTermEditor(term, from) {
  leave();
  showPage();
  view = { kind: "vocabulary" };
  let testing = false;
  const back = () => {
    if (testing) invoke("dictate_cancel").catch(() => {});
    from ? openSession(from.id) : showVocabulary();
  };
  const spelling = el("input", { value: term?.spelling ?? "", placeholder: "SIBR", "aria-label": "Spelling", autocomplete: "off", spellcheck: "false" });
  const sounds = el("input", { value: (term?.sounds_like ?? (from?.heard ? [from.heard] : [])).join(", "),
    placeholder: "cyber, sigh ber", "aria-label": "Sounds like", autocomplete: "off" });
  const letters = el("input", { type: "checkbox", checked: !!term?.letters });
  const kind = term?.when?.kind ?? "always";
  const radio = (value, label, detail) => el("label", { class: "choice" },
    el("input", { type: "radio", name: "when", value, checked: kind === value }),
    el("span", {}, label, el("small", {}, detail)));
  const near = el("input", { value: term?.when?.kind === "near" ? term.when.words.join(", ") : "", placeholder: "platform, release, dashboard",
    "aria-label": "Words it's said near", autocomplete: "off" });
  const nearField = el("div", { class: "vocab-near" }, near);

  // The speech model is the judge of what it hears: the person says the word and sees how it comes out.
  const testResult = el("div", { class: "vocab-test-result", role: "status" });
  const testLabel = el("span", {}, "Say it");
  const test = el("button", { class: "pill with-icon" }, svg("mic", 16), testLabel);
  test.addEventListener("click", async () => {
    if (!testing) {
      try {
        await invoke("dictate_start");
      } catch (e) {
        testResult.replaceChildren(el("span", { class: "danger" }, String(e)));
        return;
      }
      testing = true;
      test.classList.add("primary");
      testLabel.textContent = "Done";
      testResult.replaceChildren("Say the word the way you normally would, then click Done.");
      return;
    }
    testing = false;
    test.disabled = true;
    test.classList.remove("primary");
    testLabel.textContent = "Listening…";
    try {
      const heard = (await invoke("vocab_test_stop")).trim().replace(/[.!?,]+$/, "");
      if (!heard) {
        testResult.replaceChildren("Nothing was heard. Try again, a little closer to the microphone.");
      } else {
        const have = splitList(sounds.value).map((s) => s.toLowerCase());
        const add = el("button", { class: "link" }, "Add to Sounds like");
        add.addEventListener("click", () => {
          sounds.value = [...splitList(sounds.value), heard].join(", ");
          add.remove();
        });
        const covered = have.includes(heard.toLowerCase()) || heard.toLowerCase() === spelling.value.trim().toLowerCase();
        testResult.replaceChildren("The speech model wrote ", el("b", {}, `“${heard}”`), ". ", covered ? "That's covered already." : add);
      }
    } catch (e) {
      testResult.replaceChildren(el("span", { class: "danger" }, String(e)));
    }
    test.disabled = false;
    testLabel.textContent = "Say it again";
  });

  const save = el("button", { class: "primary pill" }, "Save");
  const page = el("div", { class: "page vocab-editor" },
    el("button", { class: "link", onclick: back }, from ? "‹ Back to the meeting" : "‹ Vocabulary"),
    el("h1", {}, term ? `Edit “${term.spelling}”` : "Add a word"),
    el("label", { class: "vocab-field" }, el("span", {}, "Spelling", el("small", {}, "Exactly how it should be written.")), spelling),
    el("label", { class: "vocab-field" }, el("span", {}, "Sounds like",
      el("small", {}, "The everyday words it's mistaken for, separated by commas.")), sounds),
    el("div", { class: "vocab-field" }, el("span", {}, "Not sure?", el("small", {}, "Say the word and see what the speech model writes.")),
      el("div", { class: "vocab-test" }, test, testResult)),
    el("label", { class: "field" }, letters, el("span", {}, "Also said letter by letter, as “S I B R”")),
    el("fieldset", { class: "vocab-when" }, el("legend", {}, "When should it replace the everyday word?"),
      radio("always", "Always", "Every time the everyday word is heard."),
      radio("near", "Only when talking about…", "When one of these words is said close by, so other uses of the everyday word are left alone."),
      nearField,
      radio("suggest", "Just suggest", "Underline it in meetings so you can accept it with a click. Dictation is left as heard.")),
    el("div", { class: "actions" }, save, el("button", { class: "pill", onclick: back }, "Cancel")));
  const syncNear = () => { nearField.hidden = page.querySelector("input[name=when]:checked")?.value !== "near"; };
  page.querySelectorAll("input[name=when]").forEach((r) => r.addEventListener("change", syncNear));
  syncNear();
  save.addEventListener("click", async () => {
    const choice = page.querySelector("input[name=when]:checked")?.value ?? "always";
    const when = choice === "near" ? { kind: "near", words: splitList(near.value) } : { kind: choice };
    save.disabled = true;
    try {
      if (testing) { testing = false; await invoke("dictate_cancel").catch(() => {}); }
      const saved = await invoke("save_vocab_term",
        { term: { ...(term ?? {}), spelling: spelling.value, sounds_like: splitList(sounds.value), letters: letters.checked, when } });
      if (from) {
        await invoke("fix_past_transcripts", { dryRun: false, only: from.id });
        openSession(from.id);
      } else {
        showVocabulary(`Saved “${saved.spelling}”.`);
      }
    } catch (e) {
      save.disabled = false;
      showError(page, e);
    }
  });
  main.replaceChildren(page);
  spelling.focus();
}

/// A line's words, with the vocabulary's changes underlined: solid where a word was changed, dotted where
/// one is only suggested.
function lineWords(seg) {
  const fixes = seg.fixes ?? [];
  if (!fixes.length) return [seg.text];
  const words = seg.text.split(" ");
  const out = [];
  let i = 0;
  fixes.forEach((f, index) => {
    if (f.at < i || f.at + f.words > words.length) return;
    if (f.at > i) out.push(words.slice(i, f.at).join(" ") + " ");
    const title = f.applied ? `Written as “${f.fixed}” from your vocabulary; heard “${f.raw}”. Click to change it back.`
      : `Did they say “${f.fixed}”? Click to use your vocabulary's spelling.`;
    out.push(el("span", { class: `vfix ${f.applied ? "applied" : "suggested"}`, "data-fix": String(index), title },
      words.slice(f.at, f.at + f.words).join(" ")));
    i = f.at + f.words;
    if (i < words.length) out.push(" ");
  });
  if (i < words.length) out.push(words.slice(i).join(" "));
  return out;
}

/// A small menu under a corrected word, to change it back or accept a suggestion.
function fixMenu(anchor, fix, run) {
  document.querySelector(".fix-menu")?.remove();
  const menu = el("div", { class: "fix-menu", role: "dialog" },
    el("span", {}, ...(fix.applied ? ["Heard “", el("b", {}, fix.raw), "”"] : ["Did they say “", el("b", {}, fix.fixed), "”?"])),
    el("button", { class: "pill", onmousedown: (e) => e.preventDefault(), onclick: () => { menu.remove(); run(); } },
      fix.applied ? `Change back to “${fix.raw}”` : `Use “${fix.fixed}”`));
  const r = anchor.getBoundingClientRect();
  menu.style.left = `${Math.max(8, Math.min(r.left, innerWidth - 288))}px`;
  menu.style.top = `${r.bottom + 6}px`;
  document.body.append(menu);
  const close = (e) => {
    if (menu.contains(e.target)) return;
    menu.remove();
    document.removeEventListener("mousedown", close, true);
  };
  setTimeout(() => document.addEventListener("mousedown", close, true));
}

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

/// A row that opens a page. Its label and value share a line while they fit; when they don't, the value
/// goes under the label, or, with `drop` (the section heading already says what the label would), the
/// label is hidden and the value has the line to itself.
function linkRow(label, value, onClick, { drop = false } = {}) {
  const row = el("button", { class: "row link-row", style: "width:100%;border-radius:0;border:none;border-bottom:1px solid var(--line);text-align:left", onclick: onClick },
    el("span", { class: "label" }, label), el("span", { class: "value" }, value ?? ""), el("span", { "aria-hidden": "true", class: "value" }, "›"));
  if (drop) row.dataset.drop = "";
  rowFit.observe(row);
  return row;
}

/// Marks a link row "tight" when its label and value don't fit on one line, again whenever its width changes.
const rowFit = new ResizeObserver((entries) => {
  for (const { target: row } of entries) {
    const width = Math.round(row.clientWidth);
    if (!width || row.fitWidth === width) continue;
    row.fitWidth = width;
    row.classList.remove("tight");
    const value = row.querySelector(":scope > .value:not([aria-hidden])");
    row.classList.toggle("tight", row.scrollWidth > row.clientWidth + 1 || (!!value && value.scrollWidth > value.clientWidth + 1));
  }
});

async function showSettings() {
  leave();
  showPage();
  view = { kind: "settings" };
  refreshItems();
  const [fresh, ready, prints, terms] = await Promise.all([invoke("get_prefs"), invoke("model_ready"), invoke("list_voiceprints").catch(() => []),
    invoke("list_vocabulary").catch(() => [])]);
  prefs = fresh;
  const page = el("div", { class: "page" }, el("h1", {}, "Settings"));
  // Each section is made here, then added in the order the page shows them.

  // About: the version, and checking for a newer one. Once a newer one is known, the button gets it instead.
  const version = el("span", { class: "value" }, update ? `Version ${update.current}` : "");
  if (!update) invoke("app_version").then((v) => (version.textContent = `Version ${v}`)).catch(() => {});
  const checkResult = el("small", { role: "status" });
  const checkButton = el("button", { class: "pill" }, "Check for updates");
  const getButton = el("button", { class: "primary pill", hidden: true }, platform === "android" ? "Download" : "Update now");
  // What's new sits centred under the download button, as Details does under the check button.
  const whatsNew = el("button", { class: "link details", type: "button", hidden: true, onclick: () => invoke("open_link", { url: update.notes }).catch(() => {}) }, "What's new");
  const updateStack = el("div", { class: "check-stack" }, getButton, whatsNew);
  // Details sits centred under the check button and opens a dialog saying what a check sends. It hides with the button.
  const details = el("button", { class: "link details", type: "button", "aria-haspopup": "dialog" }, "Details");
  details.addEventListener("click", () => showCheckDetails(details));
  const detailsWrap = el("div", { class: "details-wrap" }, details);
  const checkStack = el("div", { class: "check-stack" }, checkButton, detailsWrap);
  const showNewer = () => {
    const newer = !!update?.newer;
    checkButton.hidden = newer;
    detailsWrap.hidden = newer;
    updateStack.hidden = !newer;
    getButton.hidden = whatsNew.hidden = !newer;
    if (newer) checkResult.textContent = `Version ${update.latest} is available.`;
  };
  showNewer();
  checkButton.addEventListener("click", async () => {
    checkButton.disabled = true;
    checkResult.textContent = "Checking…";
    const found = await checkForUpdate(true);
    checkButton.disabled = false;
    checkResult.textContent = !found ? "Couldn't check. Check this device is online."
      : found.newer ? "" : `This is the latest version.`;
    showNewer();
    if (found?.newer) speak(`Version ${found.latest} is available.`);
  });
  // The download's progress shows in the sidebar's update bar, so a version put off with "Later" shows there again.
  getButton.addEventListener("click", () => {
    if (recording || dictation) return void (checkResult.textContent = "Finish the recording first, then update.");
    remember("update-later", "");
    getButton.disabled = true;
    checkResult.textContent = platform === "android" ? "Opening the download in your browser…" : "Downloading… The progress shows in the sidebar.";
    installUpdate().finally(() => { getButton.disabled = false; });
  });
  const about = [el("h2", {}, "About"), el("div", { class: "group" },
    el("div", { class: "row" }, el("span", { class: "label" }, "Voice by Superteem", version, checkResult),
      checkStack, updateStack))];

  // Optimise: offered with a callout while it would change something; the pages it leads to always.
  const callout = await optimiseCallout().catch(() => null);
  const optimise = [el("h2", {}, "Optimise"), callout, el("div", { class: "group" },
    linkRow(`Optimise for this ${isPhone() ? "phone" : "computer"}`, "Your languages, then the best models and voices for them here", () => showOptimise(OPTIMISE_FROM.settings)),
    linkRow("Local AI", `What runs on this ${isPhone() ? "phone" : "computer"}`, () => showAiSettings()))];

  const languages = [el("h2", {}, "Languages"), el("div", { class: "group" },
    linkRow("Languages", await languagesSummary(), () => showLanguages(), { drop: true }))];

  // Nothing here applies on a phone yet, so the section isn't shown there.
  const listening = isPhone() ? [] : [el("h2", {}, "Listening"), el("div", { class: "group windows-only" },
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
    }))];
  // Android's permissions, straight after About as everything else needs them.
  const phone = platform === "android" ? [el("h2", {}, "This phone"), el("div", { class: "group" },
    linkRow("App permissions", "Microphone, Bluetooth, notifications", () => showPermissions(), { drop: true }))] : [];

  const notesSection = [el("h2", {}, "Notes"), el("div", { class: "group" },
    toggleRow("Start dictating straight away", "A new note starts listening as it opens. When off, press Dictate to begin.", !prefs.dictate_manually, async (on) => {
      prefs.dictate_manually = !on;
      await invoke("save_prefs", { dictateManually: !on });
    }))];

  const meetings = [el("h2", {}, "Meetings"), el("div", { class: "group" },
    el("div", { class: "row" }, el("span", { class: "label" }, "Consent question",
      el("small", {}, "Voice always confirms everyone agreed before a meeting is recorded.")), el("span", { class: "value" }, "Always asked")),
    el("div", { class: "row" }, el("span", { class: "label" }, "Tracks used last time"),
      el("span", { class: "value" }, [...prefs.inputs, prefs.pins ? "Voice Pins" : null].filter(Boolean).join(", ") || "Default microphone")))];

  // Speech to text: its models, vocabulary and download settings, on their own page.
  const speech = await invoke("speech_models").catch(() => []);
  const inUse = speech.filter((m) => m.ready && m.used_for.length).map((m) => m.name);
  const transcriptionRow = linkRow("Transcription", ready
    ? [inUse.join(" and ") || "Speech model ready", terms.length ? `${terms.length} vocabulary word${terms.length === 1 ? "" : "s"}` : ""].filter(Boolean).join(" · ")
    : "The speech model needs downloading", () => showTranscription(), { drop: true });
  if (!ready) transcriptionRow.classList.add("needs-attention");
  const transcription = [el("h2", {}, "Transcription"), el("div", { class: "group" }, transcriptionRow)];

  const analysis = [el("h2", {}, "Analysis"), el("div", { class: "group" },
    linkRow("Model and when it runs", "The language model that makes analyses", () => showAnalysisSettings()),
    linkRow("Analysis templates", "Summary, standup, interview, SOAP and your own", () => showTemplates()))];

  const voices = [el("h2", {}, "Voices"), el("div", { class: "group" },
    linkRow("Voices", `${await readingAloudSummary()} · ${prints.length ? `${prints.length} voice print${prints.length === 1 ? "" : "s"}` : "No voice prints yet"}`, () => showVoices(), { drop: true }))];

  const theme = el("div", { class: "theme", role: "group", "aria-label": "Theme" });
  const current = document.documentElement.dataset.theme ?? "system";
  for (const [value, label] of [["system", "System"], ["light", "Light"], ["dark", "Dark"]]) {
    theme.append(el("button", { class: current === value ? "on" : "", "aria-pressed": String(current === value),
      onclick: () => { pickTheme(value); showSettings(); } }, label));
  }
  const appearance = [el("h2", {}, "Appearance"), el("div", { class: "group" }, el("div", { class: "row" }, el("span", { class: "label" }, "Theme"), theme))];

  const storage = [el("h2", {}, "Storage"), el("div", { class: "group" },
    el("div", { class: "row" }, el("span", { class: "label" }, "Recordings and notes", el("small", {}, "Kept on this device only.")),
      isDesktop() ? el("button", { class: "pill", onclick: () => invoke("open_folder", { id: null }) }, "Open folder") : null))];

  for (const section of [about, phone, optimise, languages, listening, notesSection, meetings, transcription, analysis, voices, appearance, storage]) {
    page.append(...section.filter(Boolean));
  }
  main.replaceChildren(page);
}

/// Starts the speech model download, or joins the one setup already started. `done` runs if the page is
/// still showing when it finishes.
function modelDownloadButton(done) {
  const holder = el("span");
  const button = el("button", { class: "primary pill" }, "Download (142 MB)");
  const wait = async () => {
    button.disabled = true;
    button.replaceChildren(el("span", { class: "spinner" }), el("span", { "data-model-progress": "" }, downloadText()));
    try {
      await downloadModel();
      if (holder.isConnected) done();
    } catch (e) {
      button.disabled = false;
      button.textContent = "Try the download again";
      showError(holder, e);
    }
  };
  button.addEventListener("click", wait);
  if (modelDownload) wait();
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
  // The button is only an icon, so its tooltip says the state.
  document.getElementById("listen-chip").title = on ? "Listening for “Hey Superteem”. Click to talk." : "Voice off. Click to talk.";
}

// ---------- First run ----------

let onboardEnrolling = false;
/// Set while setup has the person practise "Hey Superteem".
let setupPractising = false;

/// Cancels a voice print reading or wake word training left half done, so the microphone is released.
function cancelOnboardReading() {
  if (onboardEnrolling) {
    onboardEnrolling = false;
    invoke("stop_enrolment", { save: false }).catch(() => {});
  }
  practiceHandler = null;
  if (setupPractising) {
    setupPractising = false;
    invoke("stop_wake_practice", { print: null }).catch(() => {});
  }
}

/// Setup's pages, in order. Android can't listen for the wake word yet, so it skips saying hello. The
/// speech model page only shows while the model isn't there yet.
/// The numbers are what is saved to resume setup, so new pages get new numbers rather than renumbering.
const SETUP = { welcome: 0, voice: 1, model: 2, hello: 3, done: 4, permissions: 5 };

/// What the person agrees to by pressing "Agree and get started". Kept short on purpose.
const TERMS = [
  ["Your voice stays on this device",
    "Voice by Superteem turns speech into text on this device. Your recordings, transcripts, notes and voice print (a "
    + "numeric description of how your voice sounds, not a recording of it) are stored only here and are never sent "
    + "to us or anyone else. Checking for an update sends Superteem the app version, the operating system and its "
    + "version, the models in use, the configured languages, and whether the check was automatic, requested, or the "
    + "start of an install. That check does not include recordings, transcripts, notes or the voice print."],
  ["You're in control",
    "Delete voice prints, notes and recordings in the app at any time. Uninstalling removes the app; your files stay "
    + "in the Superteem Voice folder until you delete it."],
  ["Open source, no warranty",
    "Voice is free software under the GNU General Public License v3. You may use, study, change and share it "
    + "under that licence. It is provided as is, without warranty of any kind."],
  ["Use it lawfully",
    "You are responsible for how you use Voice. Get consent before recording anyone, follow the recording, "
    + "privacy and surveillance laws where you are, and don't use it for anything illegal. We can't prevent misuse "
    + "and aren't responsible for it."],
  ["Our liability",
    "To the extent the law allows, Superteem and its authors aren't liable for any loss or damage from using it, "
    + "including missing or wrong transcripts. Nothing here limits rights you have under the Australian Consumer Law "
    + "or other laws that can't be excluded."],
  ["Governing law",
    "These terms are governed by the laws of Victoria, Australia. If they change, Voice will ask you again."],
];

// What a check for an update sends. A dialog rather than a hover tip, so it opens by tap on a phone too.
// Closes with Close, Escape, or a tap outside it.
function showCheckDetails(opener) {
  const dialog = el("dialog", { class: "check-details", "aria-label": "Telemetry" },
    el("div", { class: "body" },
      el("h2", {}, "Telemetry"),
      el("p", {}, "Checking for an update sends telemetry data (app version, operating system, model names, and languages used) to voice.superteem.com to help provide you with the correct download. No information about you or your data is transmitted. "
        + "Voice by Superteem is built to help you secure your data on your devices and give you a better alternative to AI cloud providers that store everything about you for their own benefit. ",
        el("button", { class: "link inline", onclick: () => { dialog.close(); showTerms(); } }, "Read our privacy policy"), "."),
      el("div", { class: "actions" }, el("button", { class: "primary pill", onclick: () => dialog.close() }, "Close"))));
  dialog.addEventListener("click", (e) => { if (e.target === dialog) dialog.close(); });
  dialog.addEventListener("close", () => { dialog.remove(); opener?.focus(); });
  document.body.append(dialog);
  dialog.showModal();
}

function showTerms() {
  const dialog = el("dialog", { class: "terms", "aria-label": "Terms of use and privacy" },
    el("h2", {}, "Terms of use and privacy"),
    ...TERMS.map(([heading, text]) => el("div", {}, el("h3", {}, heading), el("p", {}, text))),
    el("div", { class: "actions" }, el("button", { class: "primary pill", onclick: () => dialog.close() }, "Close")));
  dialog.addEventListener("close", () => dialog.remove());
  document.body.append(dialog);
  dialog.showModal();
}

// ---------- The speech model download, shared by setup and Settings ----------

/// The download in progress, so setup can start it early and any page can wait on the same one.
let modelDownload = null;
let modelProgress = 0;
let modelError = null;

function downloadModel() {
  modelDownload ??= (async () => {
    modelError = null;
    const unlisten = await listen("model-download", (e) => {
      modelProgress = e.payload;
      for (const label of document.querySelectorAll("[data-model-progress]")) label.textContent = downloadText();
    });
    try {
      await invoke("download_model");
    } catch (e) {
      modelError = e;
      throw e;
    } finally {
      unlisten();
      modelDownload = null;
    }
  })();
  return modelDownload;
}

function downloadText() {
  return modelProgress ? `Downloading… ${Math.round(modelProgress * 100)}%` : "Downloading…";
}

/// The thin bar of steps at the top of setup. "The speech model" shares a step with saying hello, as it
/// is usually ready by then and skipped.
/// On Android, permissions come first.
function setupSteps(step) {
  const order = platform === "android"
    ? { [SETUP.permissions]: 0, [SETUP.voice]: 1, [SETUP.model]: 2, [SETUP.hello]: 2, [SETUP.done]: 3 }
    : { [SETUP.voice]: 0, [SETUP.model]: 1, [SETUP.hello]: 1, [SETUP.done]: 2 };
  const count = platform === "android" ? 4 : 3;
  const at = order[step] ?? 0;
  return el("div", { class: "steps", "aria-label": `Step ${at + 1} of ${count}` },
    Array.from({ length: count }, (_, i) => el("span", { class: i < at ? "done" : i === at ? "now" : "" })));
}

/// What each kind of Android access is for. The microphone is the one the app can't work without.
const PERMISSIONS = {
  microphone: ["Microphone", "Needed to record meetings, notes and your voice print.", true],
  nearby: ["Nearby devices", "For a Bluetooth headset microphone and Superteem Voice Pins.", false],
  notifications: ["Notifications", "Shows that recording is on while you use other apps, with a tap back to Voice.", false],
};

/// A live list of the app's Android permissions, each with its state and the way to allow it. Checked again
/// every couple of seconds and whenever the app comes back from Android's settings. `changed` gets whether
/// the microphone is allowed.
function permissionsPanel(changed) {
  const holder = el("div", { class: "group permissions" });
  const tips = el("div", { class: "legend permission-tips" },
    "If the microphone is allowed but recordings are silent, open Quick Settings (swipe down from the top) and check ",
    el("b", {}, "Microphone access"), " is on, and that no call or voice assistant is using the microphone.");
  let timer = null;
  const refresh = async () => {
    if (!holder.isConnected && timer) {
      clearInterval(timer);
      document.removeEventListener("visibilitychange", refresh);
      return;
    }
    const { permissions = [] } = await invoke("app_permissions").catch(() => ({}));
    holder.replaceChildren(...permissions.filter((p) => PERMISSIONS[p.id]).map((p) => {
      const [name, why, required] = PERMISSIONS[p.id];
      const state = el("span", { class: p.allowed ? "permission-state ok" : "permission-state" }, p.allowed ? "Allowed" : required ? "Needed" : "Not allowed");
      const action = p.allowed ? ""
        : p.blocked
          ? el("button", { class: "pill", onclick: () => invoke("open_app_settings").catch(() => {}) }, "Open Android settings")
          : el("button", { class: required ? "primary pill" : "pill", onclick: async () => {
            await invoke("request_app_permissions", { ids: [p.id] }).catch(() => {});
            setTimeout(refresh, 800);
          } }, "Allow");
      return el("div", { class: "row" }, el("span", { class: "label" }, name, el("small", {}, why),
        p.blocked && !p.allowed ? el("small", {}, "Android won't ask again: allow it under Permissions in the app's settings.") : ""),
        el("span", { class: "permission-action" }, state, action));
    }));
    changed?.(permissions.find((p) => p.id === "microphone")?.allowed ?? true);
  };
  refresh();
  timer = setInterval(refresh, 2000);
  document.addEventListener("visibilitychange", refresh);
  return el("div", {}, holder, tips);
}

/// Settings › Transcription: the speech models that turn speech into text (which one each language uses,
/// downloading and deleting them), vocabulary, and the download proxy and model folder.
async function showTranscription(message) {
  leave();
  showPage();
  view = { kind: "transcription" };
  refreshItems();
  const [models, terms, info, ready] = await Promise.all([invoke("speech_models").catch(() => []), invoke("list_vocabulary").catch(() => []),
    languageData(true).catch(() => null), invoke("model_ready").catch(() => false)]);
  const page = el("div", { class: "page" },
    el("button", { class: "link back-link", onclick: () => showSettings() }, "‹ Settings"),
    el("h1", {}, "Transcription"),
    el("p", { class: "hint" }, "Speech is turned into text on this device by a Whisper model, for dictation, voice commands and meeting transcripts. Nothing is sent anywhere."));
  if (message) page.append(el("div", { class: "notice" }, message));
  if (!ready) page.append(el("div", { class: "notice" }, el("strong", {}, "A speech model is needed. "), "Download one below to dictate and transcribe."));

  // The models: what each is for, which of the person's languages it transcribes, and getting or removing it.
  const list = el("div", { class: "group" });
  for (const m of models) {
    const status = el("small", {}, m.about, m.used_for.length ? ` Transcribes ${m.used_for.map(languageName).join(" and ")} now.` : "");
    const row = el("div", { class: "row" }, el("span", { class: "label" }, m.name,
      m.used_for.length && m.ready ? el("span", { class: "badge ok" }, "In use") : null,
      m.suggested && !m.ready ? el("span", { class: "badge" }, "Suits this device") : null, status));
    if (m.ready) {
      row.append(el("span", { class: "value" }, sizeText(m.size)));
      // The English model stays: it's what dictation and commands fall back on.
      if (m.multilingual) {
        row.append(el("button", { class: "link", onclick: async () => {
          try { await invoke("delete_speech_model", { id: m.id }); showTranscription(`${m.name} was deleted.`); } catch (e) { showError(page, e); }
        } }, "Delete"));
      }
    } else {
      const get = el("button", { class: m.suggested || (!ready && !m.multilingual) ? "primary pill" : "pill" }, `Download (${sizeText(m.size)})`);
      get.addEventListener("click", async () => {
        get.disabled = true;
        const off = await listen("speech-model-download", (e) => { if (e.payload.id === m.id) status.textContent = `Downloading… ${Math.round(e.payload.fraction * 100)}%`; });
        try { await invoke("download_speech_model", { id: m.id }); showTranscription(`${m.name} is ready.`); } catch (e) { get.disabled = false; status.textContent = String(e); } finally { off(); }
      });
      row.append(get);
    }
    list.append(row);
  }
  const multi = models.filter((m) => m.multilingual && m.ready);
  if (multi.length > 1) {
    const pick = el("select", { "aria-label": "Speech model for other languages" }, el("option", { value: "" }, "The best one downloaded"),
      multi.map((m) => el("option", { value: m.id, selected: info?.speech_model === m.id }, m.name)));
    pick.addEventListener("change", async () => {
      try { await invoke("save_language_settings", { speechModel: pick.value }); showTranscription(); } catch (e) { showError(page, e); }
    });
    list.append(el("div", { class: "row" }, el("span", { class: "label" }, "For languages other than English", el("small", {}, "Which multilingual model transcribes them")), pick));
  }
  page.append(el("h2", {}, "Speech models"), list,
    el("p", { class: "hint" }, "English uses the English model, unless the large one is chosen and a graphics processor runs it. Other languages need a multilingual model, offered when you first choose such a language. "
      + "Spoken commands such as “Hey Superteem” and “bookmark that” are English for now."));

  page.append(el("h2", {}, "Vocabulary"), el("div", { class: "group" },
    linkRow("Vocabulary", terms.length ? `${terms.length} word${terms.length === 1 ? "" : "s"}` : "Spell names and acronyms your way", () => showVocabulary(), { drop: true })));
  page.append(await modelHelp(() => showTranscription()));
  main.replaceChildren(page);
}

/// Settings › Voices: everything about voices in one place. How Voice reads aloud to you, and how it knows
/// your voice and the people in meetings: voice prints, the microphone, devices and spoken commands.
async function showVoices() {
  leave();
  showPage();
  view = { kind: "voices" };
  refreshItems();
  const [prints, reading] = await Promise.all([invoke("list_voiceprints").catch(() => []), readingAloudSummary()]);
  const page = el("div", { class: "page" },
    el("button", { class: "link back-link", onclick: () => showSettings() }, "‹ Settings"),
    el("h1", {}, "Voices"),
    el("p", { class: "hint" }, "How the app reads aloud to you, and how it recognises your voice for commands and the people in your meetings."));
  page.append(el("h2", {}, "Reading aloud"), el("div", { class: "group" },
    linkRow("Voice and speed", reading, () => showReadingSettings())));
  page.append(el("h2", {}, "Recognising voices"), el("div", { class: "group" },
    linkRow("Voice prints", prints.length ? `${prints.length} saved` : "None yet", () => showVoicePrints()),
    linkRow("Default microphone and voice print", prefs.enrol_input ?? "Not set up", () => showOnboarding(SETUP.voice)),
    el("div", { class: "windows-only" }, linkRow("Devices", "Test, hide and add microphones and Voice Pins", () => showDevices())),
    el("div", { class: "windows-only" }, linkRow("Wake word and commands", trainedText(prints), () => showKeyphrases()))));
  main.replaceChildren(page);
}

/// The permissions page in Settings.
function showPermissions() {
  leave();
  showPage();
  view = { kind: "permissions" };
  main.replaceChildren(el("div", { class: "page" },
    el("button", { class: "link back-link", onclick: () => showSettings() }, "‹ Settings"),
    el("h1", {}, "App permissions"),
    el("p", {}, "What Voice may use on this phone. Everything stays on the phone."),
    permissionsPanel()));
}

/// A setup page's heading, with "Skip this step" (or, run again from Settings, the way back) on the right.
function setupHeading(text, skip) {
  return el("div", { class: "setup-head" }, el("h1", {}, text), skip ?? "");
}

async function showOnboarding(step = SETUP.welcome) {
  leave();
  showPage();
  view = { kind: "onboarding", step };
  document.body.classList.add("setup");
  const rerun = prefs.onboarded;
  // Remembered, so closing the app part way through comes back to this page.
  if (!rerun) invoke("save_prefs", { setupStep: step }).catch(() => {});
  const ready = await invoke("model_ready");
  const afterModel = isPhone() ? SETUP.done : SETUP.hello;
  // The speech model page is only needed while the model isn't there yet.
  if (step === SETUP.model && ready) return showOnboarding(afterModel);
  const page = el("div", { class: "page setup-page" });
  if (step !== SETUP.welcome) page.append(setupSteps(step));
  const finish = async () => {
    cancelOnboardReading();
    if (!rerun) {
      await invoke("save_prefs", { onboarded: true, setupStep: 0 }).catch(() => {});
      prefs.onboarded = true;
    }
    showHome();
  };
  // Run again from Settings, setup can be left at any point; the first time, a step can be skipped.
  const skipTo = (to) => rerun
    ? el("button", { class: "skip-link", onclick: () => { cancelOnboardReading(); showSettings(); } }, "Back to Settings")
    : el("button", { class: "skip-link", onclick: () => { cancelOnboardReading(); showOnboarding(to); } }, "Skip this step", svg("chevron", 14));

  if (step === SETUP.welcome) {
    const agree = el("button", { class: "primary pill big" }, rerun ? "Agree" : "Agree and get started");
    agree.addEventListener("click", async () => {
      agree.disabled = true;
      await invoke("save_prefs", { acceptTerms: true }).catch(() => {});
      prefs.terms_accepted = Math.floor(Date.now() / 1000);
      rerun ? showHome() : showOnboarding(platform === "android" ? SETUP.permissions : SETUP.voice);
    });
    page.classList.add("welcome");
    page.append(
      el("h1", { class: "welcome-title" }, el("span", { class: "product" }, "Voice"), " ", el("span", { class: "by" }, "by"), " ",
        el("span", { class: "welcome-logo", role: "img", "aria-label": "Superteem" },
          el("img", { class: "logo-light-mode", src: "logo-light-mode.png", alt: "" }),
          el("img", { class: "logo-dark-mode", src: "logo-dark-mode.png", alt: "" }))),
      el("p", { class: "lead" }, "Voice notes, meetings and commands, transcribed on your own device. No cloud."),
      rerun ? el("p", {}, "There are now terms of use. Agree to them to carry on.") : "",
      el("p", { class: "legend" }, "By continuing you agree to the ",
        el("button", { class: "link inline", onclick: showTerms }, "terms of use and privacy"), "."),
      el("div", { class: "actions" }, agree));
  } else if (step === SETUP.permissions) {
    // Android asks before an app may use the microphone; without it nothing records, silently.
    const next = el("button", { class: "primary pill" }, "Continue");
    next.addEventListener("click", () => showOnboarding(SETUP.voice));
    page.append(setupHeading("Permissions", skipTo(SETUP.voice)),
      el("p", {}, "Allow Voice to use the microphone so it can record. The others are optional."),
      permissionsPanel((microphone) => {
        next.textContent = microphone ? "Continue" : "Continue without the microphone";
        next.className = microphone ? "primary pill" : "pill";
      }),
      el("div", { class: "actions" }, next));
  } else if (step === SETUP.voice) {
    // The model downloads while the voice print is made, so it's usually ready by the next page.
    if (!ready && !modelDownload) downloadModel().catch(() => {});
    const next = () => showOnboarding(SETUP.model);
    page.append(setupHeading("Your microphone and voice print", skipTo(SETUP.model)),
      el("p", { class: "setup-intro" }, "Pick the microphone you'll usually use, then read a short passage aloud so the app knows your voice."));
    const body = el("div");
    page.append(body);
    main.replaceChildren(page);
    return voiceSetup(body, next);
  } else if (step === SETUP.model) {
    page.append(setupHeading("The speech model", skipTo(afterModel)),
      el("p", {}, "Everything you say is turned into text on this device, never sent anywhere. That needs a one-time download."),
      modelError && !modelDownload ? el("div", { class: "error", role: "alert" }, String(modelError)) : "",
      el("div", { class: "actions" }, modelDownloadButton(() => {
        speak("The speech model is ready.");
        showOnboarding(afterModel);
      })),
      await modelHelp(() => showOnboarding(SETUP.model)));
  } else if (step === SETUP.hello) {
    main.replaceChildren(page);
    return sayHello(page, () => showOnboarding(SETUP.done), skipTo(SETUP.done), rerun);
  } else {
    page.classList.add("welcome", "all-set");
    page.append(
      el("div", { class: "celebrate", role: "status" }, el("span", { class: "badge" }, svg("check", 44))),
      el("h1", {}, "You're all set"),
      el("div", { class: "actions" }, el("button", { class: "primary pill big", onclick: finish }, "Start")),
      // Optional: the same as Settings › Local AI.
      el("p", { class: "legend" }, el("button", { class: "link inline", onclick: async () => { await finish(); showOptimise(); } }, "Optimise Voice for this device first"),
        " · picks the best speech and analysis models and voices for your languages"));
  }
  main.replaceChildren(page);
}

/// Help for when the speech model won't download: the proxy to use, and the folder to put it in by hand.
async function modelHelp(changed) {
  const info = await invoke("model_info").catch(() => null);
  if (!info) return "";
  const proxy = el("input", { type: "text", class: "plain", placeholder: "http://proxy.example.com:8080", value: prefs.proxy ?? "",
    "aria-label": "Download proxy", spellcheck: "false" });
  const saved = el("span", { class: "legend" });
  proxy.addEventListener("change", async () => {
    try {
      await invoke("save_prefs", { proxy: proxy.value.trim() });
      prefs.proxy = proxy.value.trim();
      changed?.();
    } catch (e) {
      saved.textContent = String(e);
    }
  });
  const using = info.proxy && info.proxy_source !== "Settings"
    ? `Now using ${info.proxy}, from ${info.proxy_source === "Windows" ? "Windows' proxy settings" : "the HTTPS_PROXY setting"}.`
    : info.proxy ? "" : "No proxy is set, so it downloads directly.";
  const copy = el("button", { class: "pill" }, "Copy download link");
  copy.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(info.url);
      copy.textContent = "Link copied";
    } catch {
      copy.textContent = info.url;
    }
  });
  return el("div", { class: "model-help" },
    el("h2", {}, "Download proxy"),
    el("div", { class: "field" }, proxy, saved),
    el("div", { class: "legend" }, platform === "windows"
      ? "Only needed if your network uses a proxy that Windows doesn't already know about. "
      : "Only needed if your network makes apps go through a proxy. ",
    "user:password@ can go before the address. ", using),
    // Android keeps the app's folder private, so the model can't be put there by hand.
    isDesktop() ? el("h2", {}, "Adding the model yourself") : "",
    isDesktop() ? el("div", { class: "legend" }, "Download ", el("code", {}, info.file), " in a browser and put it in ",
      el("code", { class: "path" }, info.folder), ". Voice picks it up straight away.") : "",
    isDesktop() ? el("div", { class: "actions" },
      el("button", { class: "pill", onclick: () => invoke("open_models_folder").catch((e) => showError(copy.parentElement, e)) }, "Open the model folder"),
      copy) : "");
}

/// How many good takes of "Hey Superteem" setup asks for, and the misses after which skipping is offered.
const PRACTICE_GOAL = 5;
const PRACTICE_STRUGGLE = 4;
/// Gets each "wake-practice" event while setup's hello page is practising.
let practiceHandler = null;
listen("wake-practice", (e) => practiceHandler?.(e.payload));

/// Setup's "Say hello" step: the person says "Hey Superteem" until five circles are ticked. Each go is
/// checked to be them saying it, not a tap or other words, and the five become their wake word.
async function sayHello(page, next, skip, rerun) {
  const [{ print }, fresh] = await Promise.all([keyphrasePrint(), invoke("get_prefs")]);
  prefs = fresh;
  const device = prefs.enrol_input;
  const steps = page.querySelector(".steps") ?? "";
  if (!print || !device) {
    page.replaceChildren(steps, setupHeading("Say hello", skip),
      el("p", {}, "Superteem listens for “Hey Superteem” while the app is open. Make your voice print first, so it learns how you say it."),
      el("div", { class: "actions" },
        el("button", { class: "primary pill", onclick: () => showOnboarding(SETUP.voice) }, "Make my voice print")));
    return;
  }
  const wake = (await invoke("list_keyphrases", { print: print.id }).catch(() => ({ phrases: [] }))).phrases.find((p) => p.id === "wake");
  const known = (wake?.takes ?? 0) >= 2;

  const mic = el("div", { class: "hello-mic off", "aria-hidden": "true" }, svg("mic", 44));
  const circles = Array.from({ length: PRACTICE_GOAL }, () => el("span"));
  const ticks = el("div", { class: "ticks", role: "img", "aria-label": `0 of ${PRACTICE_GOAL}` }, circles);
  const state = el("div", { class: "hint hello-state", "aria-live": "polite" });
  const help = el("div");
  const actions = el("div", { class: "actions hello-actions" });
  const lead = el("p", {});
  page.replaceChildren(steps, setupHeading("Say hello", skip), lead, mic, ticks, state, help, actions);

  const fill = (good) => {
    circles.forEach((c, i) => {
      const done = i < good;
      if (done && !c.classList.contains("done")) c.replaceChildren(svg("check", 18));
      c.classList.toggle("done", done);
      if (!done) c.replaceChildren();
    });
    ticks.setAttribute("aria-label", `${good} of ${PRACTICE_GOAL}`);
  };
  const stop = () => {
    practiceHandler = null;
    circles.forEach((c) => c.classList.remove("checking"));
    if (setupPractising) {
      setupPractising = false;
      invoke("stop_wake_practice", { print: null }).catch(() => {});
    }
    mic.classList.replace("on", "off");
  };

  // Listening for the wake word stays off until the person turns it on, here or in Settings.
  const finished = (message) => {
    stop();
    fill(PRACTICE_GOAL);
    lead.textContent = "Superteem knows how you say “Hey Superteem”. Turn listening on to use it now, or later in Settings. "
      + "Nothing is recorded until you ask.";
    state.textContent = message ?? "";
    const listening = toggleRow("Listen for “Hey Superteem”", "While the app is open.", prefs.wake, async (on, box) => {
      try {
        await setWake(on);
      } catch (e) {
        box.checked = !on;
        state.textContent = String(e);
      }
    });
    help.replaceChildren(el("div", { class: "group hello-listen" }, listening));
    actions.replaceChildren(el("button", { class: "primary pill", onclick: next }, "Continue"),
      el("button", { class: "pill", onclick: practise }, "Practise again"));
  };

  async function practise() {
    let good = 0;
    let misses = 0;
    fill(0);
    lead.textContent = `Say “Hey Superteem” as you normally would, then pause. Each time it's heard clearly a circle is ticked. ${PRACTICE_GOAL} ticks and it knows your voice.`;
    state.textContent = "Listening… say “Hey Superteem”";
    help.replaceChildren();
    actions.replaceChildren(el("button", { class: "pill", onclick: () => { stop(); next(); } }, "Skip this for now"));
    try {
      await invoke("start_wake_practice");
    } catch (e) {
      state.textContent = "";
      help.replaceChildren(el("div", { class: "notice", role: "alert" }, `The microphone couldn't listen: ${e}. You can skip this for now and set it up later in Settings.`));
      return;
    }
    setupPractising = true;
    mic.classList.replace("off", "on");
    practiceHandler = async (p) => {
      if (!practiceHandler) return;
      // Something was heard: the circle it would tick spins while it's checked.
      circles.forEach((c) => c.classList.remove("checking"));
      if (p.checking) {
        circles[Math.min(p.good, PRACTICE_GOAL - 1)].classList.add("checking");
        state.textContent = "Heard you. Checking…";
        return;
      }
      if (p.ok) {
        good = p.good;
        fill(Math.min(good, PRACTICE_GOAL));
        state.textContent = good >= PRACTICE_GOAL ? "That's five." : `Got it: ${good} of ${PRACTICE_GOAL}. Say it again.`;
        if (good < PRACTICE_GOAL) return;
        practiceHandler = null;
        setupPractising = false;
        mic.classList.replace("on", "off");
        state.replaceChildren(el("span", { class: "spinner" }), "Learning your voice…");
        try {
          await invoke("stop_wake_practice", { print: print.id });
          await refreshPhrasesTrained();
          speak("Got it. I know how you say it now.");
          finished("All five heard.");
        } catch (e) {
          state.textContent = `That didn't save: ${e}`;
          actions.replaceChildren(el("button", { class: "primary pill", onclick: practise }, "Try again"),
            el("button", { class: "pill", onclick: next }, "Skip this for now"));
        }
        return;
      }
      misses += 1;
      state.textContent = p.reason || "That didn't count. Say “Hey Superteem”, then pause.";
      if (misses >= PRACTICE_STRUGGLE && !help.firstChild) {
        help.replaceChildren(el("div", { class: "notice hello-struggle" },
          el("p", {}, "Having trouble? Check you're close to the microphone and the room is quiet. Or skip this for now: "
            + "you can practise later in Settings, under Wake word and commands."),
          el("button", { class: "primary pill", onclick: () => { stop(); next(); } }, "Skip this for now")));
      }
    };
  }

  if (known) finished();
  else practise();
}

/// Seconds of speech a good voice print needs, and the least that is accepted.
const VOICE_TARGET_S = 20;
const VOICE_MIN_S = 8;

/// Chooses a microphone, checks it hears speech, then records a script reading into the person's voice print.
async function voiceSetup(holder, next, problem) {
  const [{ devices, default: preferred }, fresh] = await Promise.all([invoke("list_devices"), invoke("get_prefs")]);
  prefs = fresh;
  document.querySelector(".setup-intro")?.removeAttribute("hidden");
  const prints = holder.dataset.name ? [] : await invoke("list_voiceprints").catch(() => []);
  const known = [...prints].sort((a, b) => a.created - b.created)[0]?.name;
  const name = el("input", { type: "text", class: "plain", placeholder: "Your name", "aria-label": "Your name", value: holder.dataset.name ?? known ?? "" });
  const mic = el("select", { class: "plain", "aria-label": "Microphone" }, devices.map((d) => el("option", { value: d,
    selected: d === (holder.dataset.mic ?? (devices.includes(prefs.enrol_input) ? prefs.enrol_input : preferred)) }, d)));
  const micInfo = el("span");
  const showNote = () => micInfo.replaceChildren(micNote(prefs.mics[mic.value]) ?? "");
  mic.addEventListener("change", showNote);
  showNote();
  const start = el("button", { class: "primary pill" }, "Check microphone and start");
  // replaceChildren writes null as the text "null", so missing parts are left out.
  holder.replaceChildren(...[
    problem ? el("div", { class: "notice", role: "alert" }, problem) : null,
    el("h2", {}, "Your name"), name,
    el("div", { class: "legend" }, "Shown beside what you say in meeting transcripts."),
    el("h2", {}, "Microphone"), el("div", { class: "field" }, mic, micInfo),
    devices.length ? null : el("div", { class: "error" }, "No microphones found. Connect one and come back to this step."),
    el("div", { class: "legend" }, "Your voice print is kept on this device: only a description of how your voice sounds. The recording is deleted."),
    el("div", { class: "actions" }, start)].filter(Boolean));
  start.disabled = !devices.length;
  start.addEventListener("click", async () => {
    if (!name.value.trim()) return showError(holder, "Type your name, so meetings can show it.");
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
      next();
    } catch (e) {
      voiceSetup(holder, next, `The voice print wasn't saved: ${e}`);
    }
  });

  const poll = async () => {
    const s = await invoke("recording_status").catch(() => null);
    const stream = s?.streams?.[0];
    if (!s?.recording || !stream || !onboardEnrolling) return;
    if (stream.state === "failed" || stream.state === "no headset") {
      await stopReading(false).catch(() => {});
      return voiceSetup(holder, next, `${device} could not be used: ${stream.detail || "it did not start"}. Try another microphone.`);
    }
    // Android switched to another microphone: this page checks the one chosen, so it says so instead.
    if (stream.notice_id > 0) {
      await stopReading(false).catch(() => {});
      return voiceSetup(holder, next, `${device} isn't working: ${stream.notice}. Connect it, or pick another microphone.`);
    }
    // Android is handing the app silence: the room isn't quiet, the microphone is switched off for it.
    if (stream.state === "silenced") {
      if (!status.querySelector(".mic-blocked")) {
        status.replaceChildren(el("span", { class: "danger mic-blocked" }, stream.detail), micHelp("microphone"));
      }
      return;
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
        // The passage needs the room, so the page's introduction steps aside.
        document.querySelector(".setup-intro")?.setAttribute("hidden", "");
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

// ---------- The voice agent ----------

const agentEl = document.getElementById("agent");
const agentLog = document.getElementById("agent-log");
const agentState = document.getElementById("agent-state");
const agentActions = document.getElementById("agent-actions");
const agent = {
  open: false,
  // idle, or waiting for an answer: confirm ("Did you mean…?"), topic ("…something with a note?"),
  // offer (one command at a time: "To record a new note, say yes") or choose (numbered choices).
  mode: "idle",
  pending: null, // the command a confirm or offer asks about
  topic: null, // the subject of a topic question, then of its offers
  offers: [], // commands still to offer after the pending one
  choices: [], // the commands a choose question numbers
  question: "", // what was last asked, to say again
  tries: 0, // answers not understood since it was asked
  answerTimer: null,
  forcedWake: false,
  levelTimer: null,
  closedAt: 0,
  /// When the person last closed the panel themselves.
  closedByUser: 0,
  /// Something was asked since the panel opened.
  asked: false,
  /// When the panel opened by the wake word and closed with nothing asked.
  emptyWakes: [],
  wakeRestUntil: 0,
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
  return isPhone() ? Promise.resolve() : invoke("set_wake", { on: wantWake() });
}

/// Whether a meeting recording hears voice commands.
function meetingListening() {
  return !isPhone() && (prefs.wake || phrasesTrained);
}

async function refreshPhrasesTrained() {
  phrasesTrained = (await invoke("list_keyphrases", { print: null }).catch(() => null))?.any_trained ?? phrasesTrained;
}

/// The voice panel steps back soon after a command in a meeting, so the meeting isn't taken as commands.
function stepBackSoon() {
  clearTimeout(agent.quietTimer);
  agent.quietTimer = setTimeout(() => { if (agent.open && agent.mode === "idle") closeAgent(); }, 2500);
}

/// Counts utterances, so one cut short by the next doesn't unmute the microphone while the next is spoken.
let speechCount = 0;

/// What the app is saying now, so the next reply or a stop can cut it short: { source } while a natural
/// voice's audio plays.
let speechPlaying = null;

/// Stops whatever the app is saying.
function stopSpeech() {
  window.speechSynthesis?.cancel();
  try { speechPlaying?.source?.stop(); } catch {}
  speechPlaying = null;
}

/// Says `text` aloud in the voice chosen for reading aloud: the natural voice when one reads the main
/// language, else the system's voice chosen in Settings at its speed and pitch. Resolves once it has been
/// said, or at once when nothing is said.
function speak(text) {
  // The app's voice would be on the recording, so in a meeting it answers on screen only.
  if (prefs.quiet || recording) return Promise.resolve();
  stopSpeech();
  const mine = ++speechCount;
  speaking = true;
  invoke("listen_mute", { muted: true }).catch(() => {});
  const finished = () => new Promise((resolve) => setTimeout(() => {
    if (mine === speechCount) {
      speaking = false;
      invoke("listen_mute", { muted: false }).catch(() => {});
    }
    resolve();
  }, 300));
  return speakInChosenVoice(text.replace(/[“”]/g, ""), mine).catch(() => {}).then(finished);
}

async function speakInChosenVoice(text, mine) {
  const main = languageInfo?.primary ?? "en";
  const voice = isDesktop() ? await invoke("reading_voice", { language: main }).catch(() => null) : null;
  if (mine !== speechCount) return;
  if (voice) {
    try {
      const wav = await invoke("natural_speak", { text, voice: voice.id });
      if (mine !== speechCount) return;
      const playing = {};
      speechPlaying = playing;
      return await playWav(wav, playing);
    } catch {
      // The natural voice couldn't say it (its files missing, say): the system's voice does.
      if (mine !== speechCount) return;
    }
  }
  return systemSay(text, main);
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

/// A brief message at the foot of the screen, such as the microphone being switched mid-recording.
function micToast(text) {
  if (!text) return;
  document.getElementById("mic-toast")?.remove();
  const toast = el("div", { class: "mic-toast", id: "mic-toast", role: "status" }, text);
  document.body.append(toast);
  setTimeout(() => toast.remove(), 4000);
}

function say(text, big = false) {
  agentLog.append(el("div", { class: big ? "bot big" : "bot" }, text));
  agentLog.scrollTop = agentLog.scrollHeight;
  return speak(text);
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
    if (!prefs.wake && !isPhone()) {
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

function closeAgent(byUser = false) {
  if (agent.open) {
    const now = Date.now();
    agent.closedAt = now;
    if (byUser) agent.closedByUser = now;
    if (!agent.asked) {
      agent.emptyWakes = [...agent.emptyWakes.filter((t) => now - t < EMPTY_WAKE_WINDOW_MS), now];
      if (agent.emptyWakes.length >= EMPTY_WAKES) {
        agent.wakeRestUntil = now + EMPTY_WAKE_REST_MS;
        agent.emptyWakes = [];
      }
    }
  }
  agent.asked = false;
  agent.open = false;
  agentEl.hidden = true;
  clearInterval(agent.levelTimer);
  clearTimeout(agent.quietTimer);
  // The last reply ("Opening settings.") is said to the end, unless the person closed the panel.
  if (byUser) stopSpeech();
  endDialogue();
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
  }
  if (agent.mode === "confirm" || agent.mode === "offer" || agent.mode === "topic") {
    const pending = commandById(agent.pending);
    setAgentState("Waiting for your answer: yes or no");
    agentButtons(["No", () => run("no")], [pending ? `Yes: “${pending.say}”` : "Yes", () => run("yes"), true]);
  } else if (agent.mode === "choose") {
    setAgentState("Say a number, or pick one");
    agentButtons(...agent.choices.map((id, n) => [`${n + 1}. ${commandById(id).say}`, () => run(String(n + 1)), n === 0]));
  } else if (recording) {
    // Meeting controls, set above.
  } else {
    setAgentState("Listening");
    agentButtons(["Close", () => closeAgent(true)]);
  }
  const links = agentCommands().flatMap((c, i) => [i ? " · " : "", el("a", { href: "#", "data-say": c }, `“${c}”`)]);
  document.getElementById("agent-say").replaceChildren("Say or click: ", ...links);
}

document.getElementById("agent-close").addEventListener("click", () => closeAgent(true));
document.getElementById("agent-type").addEventListener("submit", (e) => {
  e.preventDefault();
  const input = document.getElementById("agent-input");
  const text = input.value.trim();
  input.value = "";
  if (text) run(text);
});
agentEl.addEventListener("keydown", (e) => { if (e.key === "Escape") closeAgent(true); });

listen("heard", (e) => onHeard(String(e.payload)));
listen("keyphrase", (e) => onKeyphrase(e.payload));

/// Commands trained as key phrases, which act as soon as they are heard, without speech to text.
const KEYPHRASE_COMMANDS = { bookmark: "bookmark", action: "action", stop: "stop-recording" };

/// When "Hey Superteem" was last heard during a dictation, so the words after it are a command.
let wokeAt = 0;
const justWoke = () => Date.now() - wokeAt < 6000;
/// "Stop recording", "stop the note", "stop dictating" and the like.
const STOP_WORDS = /^\W*(stop|end|finish|done|close)\b.*\b(record|note|dictat|writing)/i;

/// Said by mistake, or nothing followed: the panel steps back so speech isn't transcribed for nothing.
function stepBackIfQuiet() {
  clearTimeout(agent.quietTimer);
  agent.quietTimer = setTimeout(() => { if (agent.open && agent.mode === "idle") closeAgent(); }, 10_000);
}

/// The panel isn't opened again by the wake word just after the person closed it, nor for a while after it
/// opened several times running with nothing asked, so a noise mistaken for "Hey Superteem" can't keep
/// taking over the screen. Saying a command or pressing the voice button still works.
const CLOSED_QUIET_MS = 15_000;
const EMPTY_WAKES = 3;
const EMPTY_WAKE_WINDOW_MS = 120_000;
const EMPTY_WAKE_REST_MS = 300_000;
function wakeMayOpen() {
  const now = Date.now();
  return now - agent.closedByUser > CLOSED_QUIET_MS && now > agent.wakeRestUntil;
}

/// A trained phrase, heard by the keyword spotter about a tenth of a second after it was said.
function onKeyphrase(h) {
  if (speaking || enrolment || onboardEnrolling || phraseTraining) return;
  // The Settings page shows what was heard instead of acting on it.
  if (keyphrasePage) return keyphrasePage.onHeard(h);
  if (h.rejected) return;
  // Setup isn't interrupted: its hello page checks each "Hey Superteem" itself.
  if (view.kind === "onboarding" || !prefs.onboarded) return;
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
  // "Yes" and "No" answer a question the panel is waiting on, and mean nothing otherwise.
  if (h.phrase === "yes" || h.phrase === "no") {
    if (!agent.open || agent.mode === "idle") return;
    answeredAt = Date.now();
    heardLine(h.text);
    run(h.phrase, true);
    return;
  }
  if (h.phrase === "wake") {
    if (!agent.open && !wakeMayOpen()) return;
    chime();
    // No greeting: speaking would talk over the command that usually follows straight away.
    if (!agent.open) openAgent(false);
    heardLine("Hey Superteem");
    setAgentState("Listening for a command…");
    stepBackIfQuiet();
    return;
  }
  const command = KEYPHRASE_COMMANDS[h.phrase];
  if (command) {
    if (!agent.open) openAgent(false);
    heardLine(h.text);
    doCommand(command);
  }
}

/// When a trained "Yes" or "No" was last heard, so its words from speech to text aren't taken as the
/// answer to the next question too.
let answeredAt = 0;

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
  if (view.kind === "onboarding" || !prefs.onboarded) return;
  // A dictation only listens for "Hey Superteem, stop recording"; its words are written at the tick.
  if (dictation) {
    const words = wake ? text.slice(wake.index + wake[0].length) : text;
    if ((wake || justWoke()) && STOP_WORDS.test(words)) stopNoteByVoice();
    return;
  }
  if (!agent.open && !wake) return;
  if (agent.open && Date.now() - answeredAt < 3000 && answerOf(text) !== null) return;
  // A phrase is transcribed a moment after it ends, so the sound of closing the panel arrives once it's shut.
  if (!agent.open && Date.now() - agent.closedAt < 2500) return;
  if (!agent.open && !wakeMayOpen()) return;
  const rest = wake ? text.slice(wake.index + wake[0].length).trim() : text;
  if (!agent.open) {
    openAgent(!rest);
    if (!rest) stepBackIfQuiet();
  }
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
  agent.asked = true;
  if (!agent.open) await openAgent(false);
  if (!alreadyShown) heardLine(raw);
  if (agent.mode !== "idle" && answer(raw)) return;

  const [best, ...others] = interpret(raw);
  if (best.score >= 1) return doCommand(best.id);
  const close = [best, ...others].filter((c) => c.score >= 0.5).slice(0, 3);
  if (close.length === 1) return confirm(best.id);
  // Half heard, but about one subject: ask about that, then offer its commands one at a time.
  const topic = topicOf(raw, !!recording);
  if (topic?.commands.length === 1) return confirm(topic.commands[0]);
  if (topic) {
    // The commands the words came closest to are offered first, then the most usual.
    const score = (id) => [best, ...others].find((c) => c.id === id)?.score ?? 0;
    agent.topic = topic;
    agent.offers = [...topic.commands].sort((a, b) => score(b) - score(a));
    return ask("topic", `Do you want to do something with ${topic.name}?`);
  }
  if (close.length) return choose(close.map((c) => c.id), "Did you mean one of these?");
  choose(recording ? SUGGESTED_RECORDING : SUGGESTED, "Sorry, I didn't understand that. Here are some things I can do.");
}

const commandById = (id) => COMMANDS.find((c) => c.id === id);
/// How long to wait for an answer before asking again, and then giving up.
const ANSWER_WAIT_MS = 12_000;
/// Answers not understood before the app stops asking.
const MAX_TRIES = 3;

/// Asks a question and waits for the answer. Every question can be answered by voice alone. `lead` is
/// said first but not when the question is asked again.
function ask(mode, question, lead = "") {
  agent.mode = mode;
  agent.question = question;
  agent.tries = 0;
  refreshAgentControls();
  return say(lead + question).then(waitForAnswer);
}

/// Says the question again, more simply, after an answer that wasn't understood or no answer at all.
function askAgain(why) {
  if (++agent.tries >= MAX_TRIES) {
    endDialogue();
    refreshAgentControls();
    say("Sorry, I'm having trouble understanding. I'll stop listening for now. Say Hey Superteem when you need me.")
      .then(() => { if (agent.open && agent.mode === "idle") closeAgent(); });
    return;
  }
  const simple = agent.tries >= 2;
  const answers = agent.mode === "choose" ? `Say a number from 1 to ${agent.choices.length}, or say cancel.` : "Please say just yes, or no.";
  const text = agent.mode === "topic" ? `${why} Do you want to do something with ${agent.topic.name}? ${answers}`
    : agent.mode === "choose" ? `${why} ${numbered(agent.choices)}`
    : simple ? `${why} ${answers} ${offerLine(agent.pending, false)}`
    : `${why} ${agent.question}`;
  refreshAgentControls();
  say(text).then(waitForAnswer);
}

/// Asks again when nothing is said for a while; the question isn't lost to a pause.
function waitForAnswer() {
  clearTimeout(agent.answerTimer);
  if (agent.mode === "idle" || !agent.open) return;
  const asked = agent.question;
  agent.answerTimer = setTimeout(() => {
    if (agent.open && agent.mode !== "idle" && agent.question === asked) askAgain("I didn't hear an answer.");
  }, ANSWER_WAIT_MS);
}

function endDialogue() {
  clearTimeout(agent.answerTimer);
  Object.assign(agent, { mode: "idle", pending: null, topic: null, offers: [], choices: [], question: "", tries: 0 });
}

/// "Did you mean…?" for one command.
function confirm(id) {
  agent.pending = id;
  return ask("confirm", `Did you mean ${commandById(id).say}? ${offerLine(id, false)}`);
}

/// "To record a new note, say yes. Otherwise, say no."
function offerLine(id, otherwise = true) {
  return `To ${commandById(id).offer}, say yes.${otherwise ? " Otherwise, say no." : ""}`;
}

/// Offers the next command of the topic, or says there are no more.
function nextOffer(lead = "") {
  const id = agent.offers.shift();
  if (!id) {
    const name = agent.topic?.name ?? "that";
    endDialogue();
    refreshAgentControls();
    say(`${lead}That's everything I can do with ${name}. Say what you'd like, or say help.`).then(stepBackIfQuiet);
    return;
  }
  agent.pending = id;
  ask("offer", offerLine(id), lead);
}

/// "Say 1 for Take a note, 2 for…".
function numbered(ids) {
  const parts = ids.map((id, n) => `${n + 1} for ${commandById(id).say.replace(/[?.]$/, "")}`);
  return `Say ${parts.length > 1 ? `${parts.slice(0, -1).join(", ")}, or ${parts.at(-1)}` : parts[0]}.`;
}

function choose(ids, lead) {
  agent.choices = ids;
  return ask("choose", `${lead} ${numbered(ids)}`);
}

/// Takes `raw` as the answer to the question asked. Returns false when it's a new request instead.
function answer(raw) {
  clearTimeout(agent.answerTimer);
  const reply = answerOf(raw);
  if (reply === "cancel") {
    endDialogue();
    doCommand("cancel");
    return true;
  }
  if (reply === "repeat") {
    say(agent.question).then(waitForAnswer);
    return true;
  }
  const mode = agent.mode;
  if (reply === "yes" || reply === "no") {
    const pending = agent.pending;
    if (mode === "topic") {
      if (reply === "yes") return nextOffer("Okay. "), true;
    } else if (mode === "offer") {
      if (reply === "yes") return endDialogue(), doCommand(pending), true;
      return nextOffer(), true;
    } else if (mode === "confirm") {
      if (reply === "yes") return endDialogue(), doCommand(pending), true;
    } else if (mode === "choose" && reply === "yes" && agent.choices.length === 1) {
      return doCommand(agent.choices[0]), true;
    } else if (mode === "choose" && reply === "yes") {
      return askAgain("Which one?"), true;
    }
    if (reply === "no") {
      endDialogue();
      refreshAgentControls();
      say("Okay. What would you like to do?").then(stepBackIfQuiet);
      return true;
    }
  }
  if (typeof reply === "number") {
    if (mode === "choose" && reply <= agent.choices.length) {
      const id = agent.choices[reply - 1];
      endDialogue();
      heardLine(commandById(id).say);
      doCommand(id);
      return true;
    }
    if (mode === "choose") return askAgain(`There are only ${agent.choices.length}.`), true;
  }
  // A whole request said instead of an answer is done; one half heard or not heard at all is asked again,
  // more simply, rather than losing the question.
  const [best] = interpret(raw);
  if (best.score >= 1) {
    endDialogue();
    doCommand(best.id);
    return true;
  }
  if (mode === "choose") {
    const named = agent.choices.find((id) => interpret(raw).find((c) => c.id === id)?.score >= 0.5);
    if (named && interpret(raw).filter((c) => agent.choices.includes(c.id) && c.score >= 0.5).length === 1) {
      endDialogue();
      doCommand(named);
      return true;
    }
  }
  askAgain(reply === "unsure" ? "That's okay." : "Sorry, I didn't catch that.");
  return true;
}

/// Carries out one command, by its id in COMMANDS.
async function doCommand(id) {
  agent.asked = true;
  endDialogue();
  if (id === "cancel") {
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
    say(addMarker("", "action") ? `Action item marked at ${clock(nowInRecording())}.` : "Action items are marked in meetings. Nothing is recording.");
    if (recording) stepBackSoon();
    return refreshAgentControls();
  }
  if (id === "add-note") {
    const [latest] = await invoke("list_notes", { query: "" });
    return latest ? noteByVoice(latest.id, `Okay, adding to your note ${latest.title}.`) : noteByVoice(null, "There are no notes yet. Starting a new one.");
  }
  if (id === "note") {
    return noteByVoice(null, "Okay, I'm creating a new note for you now.");
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
      : "You can say: start meeting, take a note or start dictation, add to last note, read back my last note, update the app, open settings, or cancel.");
    return refreshAgentControls();
  }
  if (id === "update") {
    if (recording || dictation) {
      say("I can't update while recording. Finish first, then ask again.");
      return refreshAgentControls();
    }
    say("Checking for a new version.");
    const found = await checkForUpdate(false, "manual");
    if (!found) {
      say("I couldn't check for updates. Check this device is online, then try again.");
    } else if (!found.newer) {
      say(`You have the latest version, ${found.current}.`);
    } else {
      agent.pending = "install-update";
      return ask("confirm", `Version ${found.latest} is available. ${offerLine("install-update")}`);
    }
    return refreshAgentControls();
  }
  if (id === "install-update") {
    if (!update?.newer) return doCommand("update");
    await say(platform === "android"
      ? `I'm opening the download in your browser. When it has downloaded, open it to install.`
      : platform === "macos"
      ? `Downloading version ${update.latest}. When its window opens, drag Voice by Superteem into Applications.`
      : `Updating to version ${update.latest}. The app will close, install the update, and open again in about a minute.`);
    closeAgent();
    installUpdate();
    return;
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
/// The prompt is said to the end before dictation starts, then a tone says it's ready, so someone who
/// can't see the screen knows when to speak, and the app's own voice isn't written into the note.
async function noteByVoice(id, intro) {
  const done = phrasesTrained ? "say “Hey Superteem, stop note”, or press the tick," : "press the tick";
  const ready = prefs.quiet || recording ? "" : " Start speaking after the tone.";
  await say(`${intro} When you've finished, ${done}.${ready}`);
  closeAgent();
  if (ready) {
    chime();
    await new Promise((r) => setTimeout(r, 300));
  }
  await openNote(id, { dictate: true });
}

// ---------- Updates ----------

/// The latest release, once checked; `newer` says whether it's newer than this app.
let update = null;
const updateBar = document.getElementById("update-bar");
/// Releases put off with "Later", and announced aloud, so neither happens twice for one version.
const remembered = (key) => { try { return localStorage.getItem(key); } catch { return null; } };
const remember = (key, value) => { try { localStorage.setItem(key, value); } catch {} };

/// Asks whether a newer version is out. Returns the release, or null when it couldn't be checked.
/// `asked` shows the bar even for a version put off before.
/// A day between automatic checks; the time of the last one survives restarts.
const UPDATE_EVERY_MS = 24 * 60 * 60 * 1000;
function lastUpdateCheck() {
  try { return Number(localStorage.getItem("update-checked")) || 0; } catch { return 0; }
}

async function checkForUpdate(asked = false, reason = asked ? "manual" : "auto") {
  try {
    update = await invoke("check_update", { reason });
  } catch {
    return null;
  }
  try { localStorage.setItem("update-checked", String(Date.now())); } catch {}
  if (asked && update.newer) remember("update-later", "");
  showUpdateBar();
  // Someone using the app by voice hears about it once, as they may never see the bar.
  if (update.newer && prefs.wake && !prefs.quiet && remembered("update-announced") !== update.latest && !recording && !dictation && !agent.open) {
    remember("update-announced", update.latest);
    speak(`A new version of Voice is available. Say Hey Superteem, update the app, to install it.`);
  }
  return update;
}

function showUpdateBar(message) {
  const show = !!update?.newer && remembered("update-later") !== update.latest;
  updateBar.hidden = !show;
  if (!show) return;
  const get = el("button", { class: "primary pill", onclick: () => installUpdate() }, platform === "android" ? "Download" : "Update now");
  updateBar.replaceChildren(
    el("span", { class: "label" }, `Version ${update.latest} is available`, message ? el("small", {}, message) : null),
    get,
    el("button", { class: "link", onclick: () => invoke("open_link", { url: update.notes }).catch(() => {}) }, "What's new"),
    el("button", { class: "link", onclick: () => { remember("update-later", update.latest); showUpdateBar(); } }, "Later"));
  return get;
}

/// Gets the newer version: on Windows the installer runs and the app closes, then opens again once it's
/// installed; on Android the download opens in the browser.
async function installUpdate() {
  if (recording || dictation) return showUpdateBar("Finish the recording first, then update.");
  const button = showUpdateBar(platform === "android" ? "Opening the download in your browser…" : "Downloading…");
  if (button) button.disabled = true;
  const progress = await listen("update-progress", (e) => {
    const label = updateBar.querySelector("small");
    if (label) label.textContent = `Downloading… ${Math.round(e.payload * 100)}%`;
  });
  try {
    await invoke("install_update");
    showUpdateBar(platform === "android" ? "When the download finishes, open it to install."
      : platform === "macos" ? "Drag Voice by Superteem into Applications in the window that opened, then open it again."
      : "Installing. The app opens again when it's done.");
  } catch (e) {
    showUpdateBar(String(e));
  } finally {
    progress();
  }
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
  if (prefs.wake && !isPhone()) {
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
    // Back to the page setup was on when the app closed. The terms are agreed before anything else.
    showOnboarding(prefs.terms_accepted && Object.values(SETUP).includes(prefs.setup_step) ? prefs.setup_step : SETUP.welcome);
  } else if (!prefs.terms_accepted) {
    showOnboarding(SETUP.welcome);
  } else {
    showHome();
    // On a phone the list is home.
    if (matchMedia("(max-width: 720px)").matches) document.body.classList.add("show-list");
  }
  // Checked a little after starting, so it doesn't slow the first page, and again whenever a day has
  // passed since the last check: an hourly look, and on coming back to the app, catch a computer that slept.
  setTimeout(checkForUpdate, 8000);
  const dueForCheck = () => { if (Date.now() - lastUpdateCheck() > UPDATE_EVERY_MS) checkForUpdate(); };
  setInterval(dueForCheck, 60 * 60 * 1000);
  document.addEventListener("visibilitychange", () => { if (!document.hidden) dueForCheck(); });
  window.addEventListener("focus", dueForCheck);
})();

// ---------- Names of icon buttons on touch screens

/// A touch screen has no hover, so holding a button that shows only an icon says what it does, the way
/// hovering does with a mouse. Letting go after that doesn't press it.
(() => {
  let timer = null;
  let tip = null;
  let held = null;
  const hide = () => { clearTimeout(timer); tip?.remove(); tip = null; };
  document.addEventListener("touchstart", (e) => {
    hide();
    const target = e.target.closest?.("button, [role=menuitem], a, label.voice-row");
    const name = target?.getAttribute("title") || (target && !target.innerText.trim() ? target.getAttribute("aria-label") : null);
    if (!name) return;
    timer = setTimeout(() => {
      held = target;
      tip = el("div", { class: "touch-tip", role: "tooltip" }, name);
      document.body.append(tip);
      const r = target.getBoundingClientRect();
      const w = tip.getBoundingClientRect().width;
      tip.style.left = `${Math.max(8, Math.min(innerWidth - w - 8, r.left + r.width / 2 - w / 2))}px`;
      tip.style.top = `${r.top > 60 ? r.top - 40 : r.bottom + 8}px`;
      navigator.vibrate?.(10);
    }, 500);
  }, { passive: true });
  const end = () => { clearTimeout(timer); if (tip) setTimeout(hide, 900); };
  document.addEventListener("touchend", end, { passive: true });
  document.addEventListener("touchcancel", hide, { passive: true });
  document.addEventListener("touchmove", () => clearTimeout(timer), { passive: true });
  // The press after a long hold only showed the name.
  document.addEventListener("click", (e) => {
    if (held && e.target.closest?.("button, [role=menuitem], a, label.voice-row") === held) { e.preventDefault(); e.stopPropagation(); }
    held = null;
  }, true);
  document.addEventListener("contextmenu", (e) => { if (tip) e.preventDefault(); });
})();
