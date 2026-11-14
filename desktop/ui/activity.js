"use strict";

/// Work that carries on whichever page is open: dictation still being written in, analyses, translations and
/// chat answers (meetings being processed show their own line). The list shows it beside each note, meeting
/// and book, and says what's ready, with a brief message, when it finished while that page wasn't open.

const activity = { analyses: { current: null, waiting: [] }, dictating: new Set() };
/// Finished while its page wasn't open, by "kind/id": { text, failed }. Gone once the page is opened.
const activityReady = new Map();
/// Names from the list, by "kind/id", so a message can say which note or meeting is ready.
const activityNames = new Map();
/// The book each note in a book is in.
const activityBooks = new Map();

const activityKey = (kind, id) => `${kind}/${id}`;
/// An analysis job's kind as the list names it.
const listKind = (jobKind) => (jobKind === "meeting" ? "session" : jobKind);

/// Whether the page for this note or meeting is the one open.
function activityViewing(kind, id) {
  if (kind === "session") return (view.kind === "session" || view.kind === "processing") && view.id === id;
  if (kind === "note") return (view.kind === "note" && view.id === id) || (view.kind === "book" && activityBooks.get(id) === view.id);
  return view.kind === kind && view.id === id;
}

/// What a job is doing, as the list says it, or null for one too small to mention.
function jobText(job, running) {
  const t = job.template;
  if (t.startsWith("label:")) return null;
  if (t === "chat") return running ? "answering…" : "question waiting";
  if (t.startsWith("translate:")) return running ? "translating…" : "translation waiting";
  return running ? "analysing…" : "analysis waiting";
}

/// What's ready once a job finishes.
function readyText(job, error) {
  const t = job.template;
  if (t === "chat") return error ? "no answer" : "answer ready";
  if (t.startsWith("translate:")) return error ? "translation failed" : "translation ready";
  return error ? "analysis failed" : "analysis ready";
}

/// What the list shows for one note or meeting, or a book (with the notes in it): { text, busy, ready }, or null.
function activityFor(kind, id, notes = null) {
  if (kind === "book") {
    for (const n of notes ?? []) {
      const one = activityFor("note", n);
      if (one) return one;
    }
    return null;
  }
  const key = activityKey(kind, id);
  if (activityViewing(kind, id)) activityReady.delete(key);
  if (kind === "note" && activity.dictating.has(id)) return { text: "writing in dictation…", busy: true };
  const mine = (j) => j && listKind(j.kind) === kind && j.id === id && jobText(j, false);
  const current = activity.analyses.current;
  if (mine(current)) return { text: jobText(current, true), busy: true };
  const waiting = activity.analyses.waiting.find(mine);
  if (waiting) return { text: jobText(waiting, false), busy: true };
  const ready = activityReady.get(key);
  return ready ? { text: ready.text, ready: true, failed: ready.failed } : null;
}

/// Called by the list as it's drawn: keeps the names and books it shows.
function activityNoted(notes, sessions, books) {
  for (const n of notes) activityNames.set(activityKey("note", n.id), n.title);
  for (const s of sessions) activityNames.set(activityKey("session", s.id), meetingTitle(s.id, s.title));
  activityBooks.clear();
  for (const b of books) for (const n of b.notes) activityBooks.set(n, b.id);
}

/// The list is drawn again only when what it would show has changed, as the queue moves often.
let activityShown = "";
function refreshActivity() {
  const now = JSON.stringify([activity.analyses.current, activity.analyses.waiting, [...activity.dictating], [...activityReady.keys()]]);
  if (now === activityShown) return;
  activityShown = now;
  refreshItems();
}

/// Says briefly that something finished on a page that isn't open, with a way to go to it.
function activityToast(kind, id, text) {
  const name = activityNames.get(activityKey(kind, id))?.trim() || (kind === "session" ? "a meeting" : "a note");
  document.getElementById("activity-toast")?.remove();
  const open = el("button", { class: "link", type: "button" }, "Open");
  const toast = el("div", { class: "activity-toast", id: "activity-toast", role: "status" },
    el("span", { class: "grow" }, `${text[0].toUpperCase()}${text.slice(1)} · ${name}`), open);
  open.addEventListener("click", () => {
    toast.remove();
    kind === "session" ? openSession(id) : openNote(id);
  });
  document.body.append(toast);
  setTimeout(() => toast.remove(), 6000);
}

/// Something finished: marked ready in the list, and said, unless its page is open.
function activityDone(kind, id, text, failed) {
  if (activityViewing(kind, id)) return;
  activityReady.set(activityKey(kind, id), { text, failed });
  activityToast(kind, id, text);
  refreshActivity();
}

listen("analysis-queue", (e) => {
  activity.analyses = e.payload;
  refreshActivity();
});
listen("analysed", (e) => {
  const job = e.payload;
  if (job.template.startsWith("label:") || job.error === "Cancelled.") return;
  activityDone(listKind(job.kind), job.id, readyText(job, job.error), !!job.error);
});
listen("processed", (e) => {
  const { id, summary } = e.payload;
  activityDone("session", id, summary?.Err ? "couldn't process" : "transcript ready", !!summary?.Err);
});
listen("note-dictated", (e) => {
  activity.dictating.delete(e.payload);
  refreshActivity();
});

/// A dictation left on note `id` is writing its words in.
async function noteDictating(id) {
  activity.dictating.add(id);
  refreshActivity();
  // It may have finished before this was told: then "note-dictated" has already been and gone.
  if (!(await invoke("dictation_writing", { id }).catch(() => false))) {
    activity.dictating.delete(id);
    refreshActivity();
  }
}

invoke("activity").then((a) => {
  activity.analyses = a.analyses;
  for (const id of a.dictating) activity.dictating.add(id);
  refreshActivity();
}).catch(() => {});
