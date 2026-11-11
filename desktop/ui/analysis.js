"use strict";

/// Analyses: a stack of template results at the top of a meeting or note, under the title. Each card can be
/// collapsed, made again or removed; action items are a checklist whose owners can be reassigned; every
/// claim has source chips that show the line it came from and go there. Also the Settings pages for the
/// language model and for templates.

icon.more = '<circle cx="5" cy="12" r="1.5" fill="currentColor" stroke="none"></circle><circle cx="12" cy="12" r="1.5" fill="currentColor" stroke="none"></circle><circle cx="19" cy="12" r="1.5" fill="currentColor" stroke="none"></circle>';
icon.cog = '<circle cx="12" cy="12" r="3"></circle><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"></path>';
icon.star = '<path d="M12 3l2.7 5.6 6.1.9-4.4 4.3 1 6.1L12 17l-5.4 2.9 1-6.1-4.4-4.3 6.1-.9z"></path>';
icon["star-filled"] = '<path d="M12 3l2.7 5.6 6.1.9-4.4 4.3 1 6.1L12 17l-5.4 2.9 1-6.1-4.4-4.3 6.1-.9z" fill="currentColor"></path>';
icon.speak = '<path d="M11 5L6 9H3v6h3l5 4z"></path><path d="M15.5 8.5a5 5 0 0 1 0 7M18.5 5.5a9 9 0 0 1 0 13"></path>';
icon["chevron-left"] = '<path d="M15 6l-6 6 6 6"></path>';
icon.sparkle = '<path d="M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8z"></path><path d="M19 15l.8 2.2L22 18l-2.2.8L19 21l-.8-2.2L16 18l2.2-.8z"></path>';

/// The page showing analyses now: { kind, id, holder, ctx }.
let analysisPage = null;
/// The meeting or note the Analysis settings were opened from, to go back to with the changes made there:
/// { kind, id }.
let analysisOrigin = null;
/// "Reading part 2 of 3…", by job.
const analysisProgress = new Map();
const jobKey = (j) => `${j.kind}/${j.id}/${j.template}`;

listen("analysis-queue", () => refreshAnalyses());
listen("analysing", (e) => {
  const step = e.payload.step === "Analysing" || !e.payload.step ? "Reading" : e.payload.step;
  analysisProgress.set(jobKey(e.payload), e.payload.parts > 1 ? `${step}: part ${e.payload.part} of ${e.payload.parts}…` : `${step}…`);
  refreshAnalyses();
  if (typeof refreshTranslation === "function") refreshTranslation(e.payload);
});
listen("analysed", (e) => {
  analysisProgress.delete(jobKey(e.payload));
  if (typeof refreshTranslation === "function") refreshTranslation(e.payload, true);
  if (e.payload.template.startsWith("translate:")) return;
  if (e.payload.error && analysisPage && analysisPage.kind === e.payload.kind && analysisPage.id === e.payload.id) {
    analysisPage.errors.set(e.payload.template, e.payload.error);
  }
  refreshAnalyses();
});

/// The stack for a meeting or note. `ctx`: { speakers: [names], go(source) }.
function analysisStack(kind, id, ctx) {
  const holder = el("section", { class: "analyses", "aria-label": "Analyses" });
  analysisPage = { kind, id, holder, ctx, errors: new Map() };
  analysisOrigin = null;
  refreshAnalyses();
  return holder;
}

async function refreshAnalyses() {
  const page = analysisPage;
  if (!page || !page.holder.isConnected && page.rendered) return;
  try {
    const data = await invoke("get_analyses", { kind: page.kind, id: page.id });
    if (analysisPage !== page) return;
    page.rendered = true;
    renderAnalyses(page, data);
  } catch (e) {
    page.holder.replaceChildren();
  }
}

const isJob = (j, page, template) => j && j.kind === page.kind && j.id === page.id && (!template || j.template === template);

function renderAnalyses(page, data) {
  const { holder } = page;
  const open = holder.querySelector(".analysis-menu .menu:not([hidden])");
  if (open) return; // Don't redraw under an open menu; the next event will.
  const running = (t) => isJob(data.queue.current, page, t);
  const waiting = (t) => data.queue.waiting.some((j) => isJob(j, page, t));
  const cards = data.entries.map((e) => analysisCard(page, e, data, running(e.key), waiting(e.key)));
  // A template asked for that has no result yet shows as a card in progress, as does a translation of one.
  const jobs = [data.queue.current, ...data.queue.waiting].filter((j) => isJob(j, page) && !j.template.startsWith("translate:"));
  const pending = [];
  for (const j of jobs) {
    if (data.entries.some((e) => e.key === j.template) || pending.some((p) => p.key === j.template)) continue;
    const [id, language] = j.template.split("@");
    const t = data.templates.find((t) => t.id === id);
    if (t) pending.push({ ...t, key: j.template, name: language ? `${t.name} · ${languageName(language)}` : t.name });
  }
  for (const t of pending) cards.push(pendingCard(page, t, running(t.key)));
  for (const [template, error] of page.errors) {
    if (data.entries.some((e) => e.key === template) || pending.some((t) => t.key === template)) continue;
    const [id, language] = template.split("@");
    const base = data.templates.find((t) => t.id === id)?.name ?? id;
    const name = language ? `${base} · ${languageName(language)}` : base;
    cards.push(el("div", { class: "analysis-card failed" }, el("div", { class: "analysis-head" }, el("strong", {}, name)),
      el("p", { class: "danger" }, String(error)),
      el("div", { class: "analysis-actions" },
        el("button", { class: "link", onclick: () => { page.errors.delete(template); runAnalysis(page, template); } }, "Try again"),
        el("button", { class: "link", onclick: () => { page.errors.delete(template); refreshAnalyses(); } }, "Dismiss"))));
  }
  holder.replaceChildren(...cards, addAnalysis(page, data));
}

function runAnalysis(page, template) {
  page.errors.delete(template);
  invoke("analyse", { kind: page.kind, id: page.id, template }).catch((e) => { page.errors.set(template, e); refreshAnalyses(); });
}

/// "Analysis templates": the first time, setting up a model; after that, the person's starred
/// templates to run, the rest below them, and a cog for the settings.
function addAnalysis(page, data) {
  const used = new Set(data.entries.map((e) => e.key));
  const busy = (t) => data.queue.waiting.some((j) => isJob(j, page, t)) || isJob(data.queue.current, page, t);
  const bar = el("div", { class: "analysis-add" });
  const label = el("span", { class: "with-icon" }, svg("sparkle", 16), "Analysis templates");
  const from = () => { analysisOrigin = { kind: page.kind, id: page.id }; };
  const settings = el("button", { class: "icon-only", title: "Analysis settings: model and starred templates", "aria-label": "Analysis settings",
    onclick: () => { from(); showAnalysisSettings(); } }, svg("cog", 16));
  if (!data.ready) {
    bar.append(el("button", { class: "link add-analysis", onclick: () => { from(); showAnalysisSettings(null, true); } }, label));
    return bar;
  }
  // data.templates holds only those for this kind of page; a starred one for the other kind is shown, unusable.
  const starred = data.starred.map((id) => data.templates.find((t) => t.id === id) ?? data.elsewhere?.find((t) => t.id === id)).filter(Boolean);
  const others = data.templates.filter((t) => !data.starred.includes(t.id));
  const choice = (t) => {
    if (!t.applies_to.includes(page.kind)) return [t.name, `For ${t.applies_to[0]}s only`, () => {}, true];
    return [t.name, busy(t.id) ? "Running" : used.has(t.id) ? "Make it again" : t.description, () => runAnalysis(page, t.id), busy(t.id)];
  };
  const choices = [...starred.map(choice)];
  if (others.length) choices.push(starred.length ? "More templates" : "Templates", ...others.map(choice));
  choices.push(["Star or edit templates…", null, () => { from(); showTemplates(); }]);
  bar.append(menuButton(label, choices, "link add-analysis"), settings);
  return bar;
}

/// A button that opens a small menu of [label, detail, run] choices.
function menuButton(label, choices, buttonClass = "icon-only") {
  const wrap = el("span", { class: "export-menu analysis-menu" });
  const button = el("button", { class: buttonClass, type: "button", "aria-haspopup": "menu", "aria-expanded": "false" }, label);
  const menu = el("div", { class: "menu", role: "menu", hidden: true });
  const close = () => { menu.hidden = true; button.setAttribute("aria-expanded", "false"); document.removeEventListener("click", outside, true); };
  const outside = (e) => { if (!wrap.contains(e.target)) close(); };
  button.addEventListener("click", (e) => {
    e.stopPropagation();
    if (!menu.hidden) return close();
    menu.hidden = false;
    button.setAttribute("aria-expanded", "true");
    document.addEventListener("click", outside, true);
    menu.querySelector("button")?.focus();
  });
  menu.addEventListener("keydown", (e) => { if (e.key === "Escape") { close(); button.focus(); } });
  for (const c of choices) {
    // A string between choices labels the ones after it.
    if (typeof c === "string") { menu.append(el("div", { class: "menu-label" }, c)); continue; }
    const [text, detail, run, disabled] = c;
    const item = el("button", { type: "button", role: "menuitem", disabled: !!disabled }, el("strong", {}, text), detail ? el("small", {}, detail) : null);
    item.addEventListener("click", () => { close(); run(); });
    menu.append(item);
  }
  wrap.append(button, menu);
  return wrap;
}

function pendingCard(page, t, running) {
  const key = t.key ?? t.id;
  const progress = analysisProgress.get(jobKey({ kind: page.kind, id: page.id, template: key }));
  return el("div", { class: "analysis-card pending" },
    el("div", { class: "analysis-head" }, el("span", { class: "spinner" }), el("strong", {}, t.name),
      el("span", { class: "hint" }, running ? progress ?? "Loading the model…" : waitingText()),
      el("span", { class: "grow" }),
      el("button", { class: "link", onclick: () => invoke("cancel_analysis", { kind: page.kind, id: page.id, template: key }) }, "Cancel")));
}

const waitingText = () => (recording ? "Waiting until the recording stops" : "Waiting its turn");

/// One line summing up a collapsed card: its opening sentence, then how many of each list.
function analysisDigest(def, result) {
  const parts = [];
  const first = def.fields.find((f) => f.type === "text" && result[f.key]);
  if (first) {
    const text = String(result[first.key]);
    const sentence = text.split(/(?<=[.!?])\s/)[0];
    parts.push(sentence.length > 90 ? `${sentence.slice(0, 88)}…` : sentence);
  }
  for (const f of def.fields.filter((f) => f.type === "object-list")) {
    const items = (result[f.key] ?? []).filter((i) => !i._removed);
    if (!items.length) continue;
    const done = items.filter((i) => i._done).length;
    parts.push(`${items.length} ${fieldLabel(f).toLowerCase()}${done ? ` · ${done} done` : ""}`);
  }
  return parts.join(" · ");
}

function fieldLabel(f) {
  if (f.label) return f.label;
  const words = f.key.replace(/[_-]/g, " ");
  return words.charAt(0).toUpperCase() + words.slice(1);
}

function analysisCard(page, e, data, running, waiting) {
  const def = e.definition;
  // The stack's name for it: the template, with the language after an @ for a translation of another.
  const template = e.key ?? e.template.id;
  const card = el("div", { class: `analysis-card${e.collapsed ? " collapsed" : ""}` });
  const edit = async (change) => {
    try {
      await invoke("edit_analysis", { kind: page.kind, id: page.id, template, edit: change });
    } catch (err) {
      showError(card, err);
    }
    refreshAnalyses();
  };
  const remove = () => invoke("remove_analysis", { kind: page.kind, id: page.id, template }).then(refreshAnalyses).catch((err) => showError(card, err));
  const toggle = el("button", { class: "icon-only chevron", "aria-expanded": String(!e.collapsed), title: e.collapsed ? "Show all" : "Collapse",
    onclick: () => edit({ op: "collapse", collapsed: !e.collapsed }) }, svg(e.collapsed ? "chevron" : "down", 16));
  const progress = analysisProgress.get(jobKey({ kind: page.kind, id: page.id, template }));
  const badges = [];
  if (running) badges.push(el("span", { class: "badge busy" }, el("span", { class: "spinner" }), progress ?? "Making again…"));
  else if (waiting) badges.push(el("span", { class: "badge busy" }, waitingText()));
  else if (e.stale) badges.push(el("span", { class: "badge stale", title: "The transcript was edited after this was made" }, "Transcript changed"));
  else if (e.template_changed) badges.push(el("span", { class: "badge stale", title: "The template was edited after this was made" }, "Template changed"));
  const error = page.errors.get(template);
  const regenerate = el("button", { class: "icon-only", title: running || waiting ? "Cancel" : "Make again", "aria-label": running || waiting ? "Cancel" : "Make again",
    disabled: !data.ready && !(running || waiting),
    onclick: () => (running || waiting ? invoke("cancel_analysis", { kind: page.kind, id: page.id, template }) : runAnalysis(page, template)) },
    svg(running || waiting ? "close" : "refresh", 16));
  const index = data.entries.indexOf(e);
  // Into the person's other languages, made from the entry it was translated from.
  const written = e.language || "en";
  const into = (languageInfo?.chosen ?? []).filter((c) => c !== written && !(e.via === "translated" && c === "en"));
  const translations = into.map((c) => [`Translate into ${languageName(c)}`, data.entries.some((x) => x.key === `${e.template.id}@${c}`) ? "Make it again" : null,
    () => runAnalysis(page, `${e.template.id}@${c}`), !data.ready]);
  const more = menuButton(svg("more", 16), [
    ["Copy as text", null, () => navigator.clipboard?.writeText(card.querySelector(".analysis-body")?.innerText ?? "")],
    ...translations,
    ["Move up", null, () => invoke("move_analysis", { kind: page.kind, id: page.id, template, by: -1 }).then(refreshAnalyses), index === 0],
    ["Move down", null, () => invoke("move_analysis", { kind: page.kind, id: page.id, template, by: 1 }).then(refreshAnalyses), index === data.entries.length - 1],
    ["Remove", "Edits and ticks here are lost", remove],
  ]);
  more.querySelector("button").setAttribute("aria-label", "More");
  const head = el("div", { class: "analysis-head" }, toggle,
    el("button", { class: "analysis-name", onclick: () => edit({ op: "collapse", collapsed: !e.collapsed }) }, e.template.name,
      e.language ? el("span", { class: "badge language", title: e.via === "translated" ? "Machine-translated" : "Written in this language" }, languageName(e.language)) : null),
    e.collapsed && def ? el("span", { class: "digest" }, analysisDigest(def, e.result)) : null,
    // Minimised, it can still be read aloud; making it again waits until it's open.
    ...badges, el("span", { class: "grow" }), def ? readAloudButton(() => analysisSpeech(card), "Read this aloud", { lang: () => e.language || (e.via ? "en" : page.ctx.lang?.()) }) : null,
    e.collapsed && !(running || waiting) ? null : regenerate, more);
  card.append(head);
  if (error) card.append(el("p", { class: "danger analysis-error" }, `Couldn't make it again: ${error}`));
  if (!def) {
    if (e.collapsed) return card;
    card.append(el("p", { class: "hint" }, "Its template is no longer available, so it can only be removed."));
    return card;
  }
  // Drawn when minimised too, hidden, so it can be read aloud.
  const body = el("div", { class: "analysis-body", hidden: e.collapsed });
  if (def.notice) body.append(el("div", { class: "notice" }, def.notice));
  const chips = (evidence) => sourceChips(evidence, e.sources, page.ctx);
  def.fields.forEach((f, i) => {
    if (f.render === "hidden") return;
    const value = e.result[f.key];
    const section = el("div", { class: "analysis-section" });
    if (!(i === 0 && f.type === "text")) section.append(el("h3", {}, fieldLabel(f)));
    if (f.type === "object-list") section.append(objectList(f, value ?? [], chips, edit, page));
    else if (f.type === "list") section.append(stringList(f, value ?? [], edit));
    else section.append(scalarEditor(f, value, (v) => edit({ op: "set", field: f.key, value: v })));
    body.append(section);
  });
  const marks = (e.result.bookmarks ?? []).filter((b) => !b._removed);
  if (marks.length) {
    body.append(el("div", { class: "analysis-section" }, el("h3", {}, "Bookmarks"),
      el("ul", { class: "analysis-points" }, marks.map((b) => {
        const index = e.result.bookmarks.indexOf(b);
        return el("li", { class: b._check ? "check" : "" },
          el("span", { class: "mono bookmark-time" }, clock(b.at_s), b.note ? ` “${b.note}”` : ""),
          editable(b.point, (v) => edit({ op: "set", field: "bookmarks", index, key: "point", value: v })),
          chips(b.evidence));
      }))));
  }
  if (e.collapsed) {
    card.append(body);
    return card;
  }
  const made = e.engine.kind === "local" ? "this device" : e.engine.host ?? "a server";
  const how = e.via === "translated" ? ` · Machine-translated into ${languageName(e.language)}` : e.via === "translation" ? " · Made via English translation" : "";
  const footer = el("div", { class: "analysis-foot" },
    `Made on ${made} with ${e.engine.model}${how} · ${dateText(e.created)} · Check it against the ${page.kind === "note" ? "note" : "transcript"}.`);
  if (e.warnings?.length) {
    const list = el("ul", { class: "analysis-warnings", hidden: true }, e.warnings.map((w) => el("li", {}, w)));
    footer.append(" ", el("button", { class: "link inline", onclick: () => { list.hidden = !list.hidden; } },
      `${e.warnings.length} change${e.warnings.length === 1 ? "" : "s"} from checking`), list);
  }
  card.append(body, footer);
  return card;
}

/// Text the person can correct in place; saved when they leave it.
function editable(text, save, tag = "span") {
  const node = el(tag, { class: "editable", contenteditable: "plaintext-only", spellcheck: "true" }, text ?? "");
  node.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); node.blur(); }
    if (e.key === "Escape") { node.textContent = text ?? ""; node.blur(); }
  });
  node.addEventListener("blur", () => {
    const value = node.textContent.trim();
    if (value !== (text ?? "")) save(value);
  });
  return node;
}

function scalarEditor(f, value, save) {
  if (f.type === "enum") {
    const select = el("select", { class: "analysis-enum" }, el("option", { value: "" }, "—"),
      f.values.map((v) => el("option", { value: v, selected: v === value }, v)));
    select.addEventListener("change", () => save(select.value || null));
    return select;
  }
  if (f.type === "bool") {
    const box = el("input", { type: "checkbox", checked: !!value });
    box.addEventListener("change", () => save(box.checked));
    return el("label", { class: "analysis-bool" }, box, fieldLabel(f));
  }
  if (f.type === "when") return whenChip(value, save);
  return editable(value ?? "", save, "p");
}

function stringList(f, values, edit) {
  if (!values.length) return el("p", { class: "hint" }, "None.");
  return el("ul", { class: "analysis-points" }, values.map((v, i) => el("li", {},
    editable(v, (text) => { const next = [...values]; next[i] = text; if (!text) next.splice(i, 1); edit({ op: "set", field: f.key, value: next }); }))));
}

function whenChip(when, save) {
  const text = when ? (when.kind === "trigger" ? `when ${when.text}` : when.text) : "";
  const chip = el("button", { class: `when-chip${when ? "" : " empty"}`, type: "button", title: "When: a deadline, or what starts it" }, text || "+ when");
  chip.addEventListener("click", () => {
    const kind = el("select", {}, el("option", { value: "deadline", selected: when?.kind !== "trigger" }, "By"),
      el("option", { value: "trigger", selected: when?.kind === "trigger" }, "When"));
    const input = el("input", { type: "text", value: when?.text ?? "", placeholder: "Friday, next sprint, legal confirms…", maxlength: "80" });
    const form = el("span", { class: "when-edit" }, kind, input);
    const done = () => {
      const value = input.value.trim();
      const next = value ? { kind: kind.value, text: value } : null;
      if (JSON.stringify(next) !== JSON.stringify(when ?? null)) save(next);
      else form.replaceWith(chip);
    };
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") { e.preventDefault(); done(); }
      if (e.key === "Escape") form.replaceWith(chip);
    });
    form.addEventListener("focusout", (e) => { if (!form.contains(e.relatedTarget)) done(); });
    chip.replaceWith(form);
    input.focus();
  });
  return chip;
}

/// The owner chip: speakers here, then voice prints, then someone else by name, or nobody.
function whoChip(who, page, save) {
  const chip = el("button", { class: `who-chip${who === "Unassigned" ? " unassigned" : ""}`, type: "button", title: "Who does it" }, who || "Unassigned");
  chip.addEventListener("click", async (event) => {
    event.stopPropagation();
    const prints = await invoke("list_voiceprints").catch(() => []);
    const names = [...new Set(page.ctx.speakers ?? [])];
    const menu = el("div", { class: "menu who-menu", role: "menu" });
    const pick = (name, print) => { close(); save(name, print); };
    const close = () => { menu.remove(); document.removeEventListener("click", outside, true); };
    const outside = (e) => { if (!menu.contains(e.target)) close(); };
    const option = (label, run) => el("button", { type: "button", role: "menuitem", onclick: run }, label);
    if (names.length) menu.append(el("div", { class: "menu-label" }, page.kind === "note" ? "You" : "In this meeting"), ...names.map((n) => option(n, () => pick(n, null))));
    const others = prints.filter((p) => !names.includes(p.name));
    if (others.length) menu.append(el("div", { class: "menu-label" }, "Voice prints"), ...others.map((p) => option(p.name, () => pick(p.name, p.id))));
    const input = el("input", { type: "text", placeholder: "Someone else…", maxlength: "40" });
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && input.value.trim()) { e.preventDefault(); pick(input.value.trim(), null); }
      if (e.key === "Escape") close();
    });
    menu.append(el("div", { class: "menu-label" }, "Someone else"), input, option("Unassigned", () => pick("Unassigned", null)));
    chip.parentElement.append(menu);
    document.addEventListener("click", outside, true);
    menu.querySelector("button, input")?.focus();
  });
  return el("span", { class: "who-wrap" }, chip);
}

function objectList(f, items, chips, edit, page) {
  const visible = items.map((item, index) => ({ item, index })).filter(({ item }) => !item._removed);
  const sub = f.items ?? [];
  const primary = sub.find((s) => s.type === "text");
  const person = sub.find((s) => s.type === "person" || s.type === "speaker");
  const refs = sub.find((s) => s.type === "refs");
  const others = sub.filter((s) => s !== primary && s !== person && s !== refs);
  const set = (index, key, value) => edit({ op: "set", field: f.key, index, key, value });
  const list = el("ul", { class: `analysis-points${f.render === "checklist" ? " checklist" : ""}` });
  for (const { item, index } of visible) {
    const row = el("li", { class: `${item._check ? "check" : ""}${item._done ? " done" : ""}` });
    if (f.render === "checklist") {
      const box = el("input", { type: "checkbox", checked: !!item._done, "aria-label": "Done" });
      box.addEventListener("change", () => edit({ op: "done", field: f.key, index, done: box.checked }));
      row.append(box);
    }
    if (person) {
      row.append(whoChip(item[person.key], page, (name, print) => {
        set(index, person.key, name);
        if (print || item._who_print) set(index, "_who_print", print);
      }));
    }
    if (primary) row.append(editable(item[primary.key], (v) => set(index, primary.key, v)));
    for (const o of others) {
      if (o.type === "when") row.append(whenChip(item[o.key], (v) => set(index, o.key, v)));
      else if (o.type === "enum") row.append(el("span", { class: "field-tag" }, fieldLabel(o), ": ", scalarEditor(o, item[o.key], (v) => set(index, o.key, v))));
      else if (o.type === "text") {
        if (item[o.key] || f.render === "table") row.append(el("span", { class: "field-extra" }, el("small", {}, fieldLabel(o)), editable(item[o.key] ?? "", (v) => set(index, o.key, v || null))));
      } else row.append(el("span", { class: "field-tag" }, scalarEditor(o, item[o.key], (v) => set(index, o.key, v))));
    }
    if (refs) row.append(chips(item[refs.key] ?? []));
    if (item._check) row.append(el("span", { class: "badge stale", title: "This doesn't share any words with the lines it cites. Check it." }, "check"));
    row.append(el("button", { class: "icon-only row-delete", title: "Delete", "aria-label": "Delete", onclick: () => edit({ op: "remove", field: f.key, index }) }, svg("close", 14)));
    list.append(row);
  }
  const wrap = el("div", {}, visible.length ? list : el("p", { class: "hint" }, "None."));
  if (f.render === "checklist" && primary) {
    const blank = {};
    for (const s of sub) blank[s.key] = s.type === "refs" ? [] : s.type === "person" || s.type === "speaker" ? (page.kind === "note" ? "Me" : "Unassigned") : s.type === "text" ? "" : null;
    blank[primary.key] = "New task";
    wrap.append(el("button", { class: "link add-row", onclick: () => edit({ op: "add", field: f.key, item: blank }) }, "+ Add task"));
  }
  return wrap;
}

/// Chips for an item's sources: hovering shows the line with its neighbours, clicking goes there. On a
/// touch screen the first tap shows it, with a button to go.
function sourceChips(evidence, sources, ctx) {
  const wrap = el("span", { class: "sources" });
  for (const anchor of evidence ?? []) {
    const source = sources?.[anchor];
    if (!source) {
      wrap.append(el("span", { class: "source-chip gone", title: "That line was changed or removed" }, "source changed"));
      continue;
    }
    const chip = el("button", { class: "source-chip mono", type: "button", "aria-label": `Source at ${source.label}` }, source.label);
    let pop = null;
    const show = () => {
      if (pop) return;
      pop = el("div", { class: "source-pop", role: "tooltip" },
        source.before ? el("p", { class: "dim" }, source.before) : null,
        el("p", {}, el("strong", {}, source.speaker, source.at_s != null ? ` · ${source.label}` : ""), " ", source.text),
        source.after ? el("p", { class: "dim" }, source.after) : null,
        el("button", { class: "link inline", type: "button", onclick: (e) => { e.stopPropagation(); hide(); ctx.go?.(source); } }, "Go there"));
      chip.append(pop);
    };
    const hide = () => { pop?.remove(); pop = null; };
    let touch = false;
    chip.addEventListener("pointerdown", (e) => { touch = e.pointerType === "touch"; });
    chip.addEventListener("mouseenter", () => { if (!touch) show(); });
    chip.addEventListener("mouseleave", () => { if (!touch) hide(); });
    chip.addEventListener("focus", show);
    chip.addEventListener("blur", (e) => { if (!chip.contains(e.relatedTarget)) hide(); });
    chip.addEventListener("click", (e) => {
      if (e.target.closest(".source-pop")) return;
      if (touch && !pop) return show();
      hide();
      ctx.go?.(source);
    });
    wrap.append(chip);
  }
  return wrap;
}

// ---------- Settings › Analysis

const gb = (bytes) => `${(bytes / 1e9).toFixed(bytes >= 1e10 ? 0 : 1)} GB`;
let llmProgress = null; // { id, fraction }
listen("llm-download", (e) => {
  llmProgress = e.payload;
  document.querySelectorAll("[data-llm-progress]").forEach((n) => {
    if (n.dataset.llmProgress === e.payload.id) n.textContent = `${Math.round(e.payload.fraction * 100)}%`;
  });
});

/// `setup` the first time, from a meeting or note: the page leads with choosing a model.
async function showAnalysisSettings(message, setup = false) {
  leave();
  showPage();
  view = { kind: "analysis-settings" };
  refreshItems();
  const status = await invoke("llm_status");
  const page = el("div", { class: "page" }, backLinks("‹ Settings", () => showSettings()), el("h1", {}, "Analysis"));
  page.append(el("p", { class: "hint" }, "Summaries, decisions, action items and other notes are made from transcripts by a small language model that runs on this device. ",
    "Download one once; after that it works offline and nothing is sent anywhere."));
  if (message) page.append(el("div", { class: "notice" }, message));
  if (setup && !status.installed.length && !status.server) {
    page.append(el("div", { class: "notice" }, el("strong", {}, "First, choose a model. "),
      "It's a one-off download; after that, summaries and action items are made on this device, offline. Qwen3 1.7B is a good start. ",
      "Then star the templates you use most, below, and they'll be one click away on every meeting and note."));
  }
  const { device } = status;
  page.append(el("p", { class: "hint" }, `This ${isPhone() ? "phone" : "computer"} has ${device.ram_gb.toFixed(0)} GB of memory, so models read up to ${device.context.toLocaleString()} tokens at a time${device.too_small ? ". That's too little to run a model well here; a local server (below) can do it instead." : "."}`));

  page.append(el("div", { class: "group" }, device.gpu
    ? toggleRow("Use the graphics processor", `${device.gpu}. Summaries, translations${platform === "windows" ? " and transcripts" : ""} are made several times faster there.`, status.use_gpu, async (on) => {
      await invoke("save_voice_settings", { useGpu: on });
    })
    : el("div", { class: "row" }, el("span", { class: "label" }, "Graphics processor",
      el("small", {}, isDesktop() && platform === "windows"
        ? "None found that models can use, so they run on the processor. Updating the graphics driver can help."
        : "None that models can use here, so they run on the processor.")), el("span", { class: "value" }, "Processor"))));
  const installed = new Map(status.installed.map((i) => [i.file, i]));
  const group = el("div", { class: "group" });
  for (const m of status.catalogue) {
    const have = installed.has(m.file);
    if (!m.offered && !have) continue;
    const chosen = status.chosen === m.file && !status.server;
    const label = el("span", { class: "label" }, m.name, chosen ? el("span", { class: "badge ok" }, "In use") : null,
      el("small", {}, `${m.about} ${gb(m.size)} · ${m.licence}${device.ram_gb < m.min_ram_gb ? " · May be slow with this much memory" : ""}`));
    const row = el("div", { class: "row" }, label);
    if (status.downloading === m.id) {
      row.append(el("span", { class: "value" }, el("span", { class: "spinner" }), " ", el("span", { "data-llm-progress": m.id }, llmProgress?.id === m.id ? `${Math.round(llmProgress.fraction * 100)}%` : "Starting…")),
        el("button", { class: "pill", onclick: () => invoke("cancel_llm_download") }, "Stop"));
    } else if (have) {
      if (!chosen) row.append(el("button", { class: "pill", onclick: async () => { await invoke("save_analysis_settings", { model: m.file, server: "" }); showAnalysisSettings(); } }, "Use"));
      row.append(el("button", { class: "icon-only danger", title: "Delete", "aria-label": `Delete ${m.name}`, onclick: async () => {
        try { await invoke("delete_llm", { file: m.file }); showAnalysisSettings(); } catch (e) { showError(page, e); }
      } }, svg("trash", 16)));
    } else {
      row.append(el("button", { class: "primary pill", disabled: !!status.downloading, onclick: async () => {
        const run = invoke("download_llm", { id: m.id });
        setTimeout(() => showAnalysisSettings(), 150);
        try { await run; if (view.kind === "analysis-settings") showAnalysisSettings(`${m.name} is ready.`); }
        catch (e) { if (view.kind === "analysis-settings") showAnalysisSettings(String(e)); }
      } }, `Download (${gb(m.size)})`));
    }
    group.append(row);
  }
  for (const i of status.installed.filter((i) => !i.catalogued)) {
    const chosen = status.chosen === i.file && !status.server;
    const row = el("div", { class: "row" }, el("span", { class: "label" }, i.file, chosen ? el("span", { class: "badge ok" }, "In use") : null,
      el("small", {}, `Added by you · ${gb(i.size)} · Not checked by Superteem`)));
    if (!chosen) row.append(el("button", { class: "pill", onclick: async () => { await invoke("save_analysis_settings", { model: i.file, server: "" }); showAnalysisSettings(); } }, "Use"));
    row.append(el("button", { class: "icon-only danger", title: "Delete", "aria-label": `Delete ${i.file}`, onclick: async () => {
      try { await invoke("delete_llm", { file: i.file }); showAnalysisSettings(); } catch (e) { showError(page, e); }
    } }, svg("trash", 16)));
    group.append(row);
  }
  if (isDesktop()) {
    group.append(el("div", { class: "row" }, el("span", { class: "label" }, "Your own model",
      el("small", {}, "Put any instruct model in GGUF format in the models folder and it appears here.")),
      el("button", { class: "pill", onclick: () => invoke("open_llm_folder").catch((e) => showError(page, e)) }, "Open folder"),
      el("button", { class: "pill", onclick: () => showAnalysisSettings() }, "Refresh")));
  }
  page.append(el("h2", {}, "Model"), group);

  page.append(el("h2", {}, "When to run"), el("div", { class: "group" },
    toggleRow("Summarise meetings when they're processed", "Runs the Summary template in the background after each meeting. It waits while you record.", status.auto, async (on) => {
      await invoke("save_analysis_settings", { auto: on });
    }),
    linkRow("Analysis templates", "Star the ones you use, and make your own", () => showTemplates())));

  // A server the person runs: never used unless set here, and anything not on this computer is named first.
  const url = el("input", { type: "url", value: status.server, placeholder: "http://localhost:11434/v1", "aria-label": "Server address" });
  const model = el("input", { type: "text", value: status.server_model, placeholder: "qwen3:1.7b", "aria-label": "Model name on the server" });
  const confirmBox = el("div", { class: "notice confirm", hidden: true });
  const saveServer = async () => {
    try {
      await invoke("save_analysis_settings", { server: url.value.trim(), serverModel: model.value.trim() });
      showAnalysisSettings(url.value.trim() ? "Analyses now run on that server." : "Analyses run on this device.");
    } catch (e) {
      showError(page, e);
    }
  };
  const save = el("button", { class: "pill", onclick: () => {
    const address = url.value.trim();
    const local = !address || /^https?:\/\/(localhost|127\.|\[::1\])/i.test(address);
    if (local) return saveServer();
    const host = address.replace(/^https?:\/\//, "").split("/")[0];
    confirmBox.replaceChildren(el("p", {}, `Transcripts and notes will be sent to ${host} to be analysed. They leave this device. Only do this for a server you trust.`),
      el("button", { class: "primary danger-fill", onclick: saveServer }, "Send them there"),
      el("button", { onclick: () => { confirmBox.hidden = true; } }, "Cancel"));
    confirmBox.hidden = false;
  } }, "Save");
  page.append(el("h2", {}, "Run on a local server instead"), el("div", { class: "group" },
    el("div", { class: "row server-row" }, el("span", { class: "label" }, "Server",
      el("small", {}, "An OpenAI-compatible server you run, such as Ollama or LM Studio. Leave empty to use this device.")),
    el("div", { class: "server-fields" }, url, model, save))), confirmBox);
  main.replaceChildren(page);
}

/// The back link, and when the settings were opened from a meeting or note, a way straight back to it.
function backLinks(text, back) {
  const links = el("div", { class: "back-links" }, el("button", { class: "link back-link", onclick: back }, text));
  if (analysisOrigin) {
    const { kind, id } = analysisOrigin;
    links.append(el("button", { class: "pill with-icon back-to", onclick: () => (kind === "note" ? openNote(id) : openSession(id)) },
      svg("chevron-left", 14), `Back to the ${kind}`));
  }
  return links;
}

// ---------- Settings › Templates

async function showTemplates(message) {
  leave();
  showPage();
  view = { kind: "templates" };
  refreshItems();
  const { templates: all, starred } = await invoke("list_templates");
  const page = el("div", { class: "page" }, backLinks("‹ Analysis", () => showAnalysisSettings()), el("h1", {}, "Analysis templates"));
  page.append(el("p", { class: "hint" }, "A template says what to pull out of a meeting or note and how to lay it out. Star the ones you use: they're offered first under Analysis templates on every meeting and note. Each one stacks there and can be made again."));
  if (message) page.append(el("div", { class: "notice" }, message));
  const group = el("div", { class: "group" });
  for (const t of all) {
    const kinds = t.applies_to.length === 2 ? "Meetings and notes" : t.applies_to[0] === "note" ? "Notes" : "Meetings";
    const label = el("span", { class: "label" }, t.name,
      t.source !== "built-in" ? el("span", { class: "badge" }, t.source === "customised" ? "Customised" : "Yours") : null,
      el("small", {}, t.errors.length ? "" : `${t.description} · ${kinds}${t.manual_only ? " · Only when asked" : ""}`));
    if (t.errors.length) label.append(el("ul", { class: "danger template-errors" }, t.errors.map((e) => el("li", {}, e))));
    const on = starred.includes(t.id);
    const star = el("button", { class: `icon-only star${on ? " on" : ""}`, title: on ? "Starred: offered first on meetings and notes" : "Star it, to offer it first",
      "aria-label": on ? `Unstar ${t.name}` : `Star ${t.name}`, "aria-pressed": String(on), disabled: !!t.errors.length,
      onclick: async () => { await invoke("star_template", { id: t.id, starred: !on }).catch((e) => showError(page, e)); showTemplates(); } }, svg(on ? "star-filled" : "star", 18));
    const row = el("div", { class: "row" }, star, label);
    row.append(el("button", { class: "pill", onclick: () => editTemplate(t) }, t.file ? "Edit" : "Customise"));
    if (t.file) {
      row.append(el("button", { class: "icon-only danger", title: t.source === "customised" ? "Back to the built-in" : "Delete", "aria-label": "Delete",
        onclick: async () => { await invoke("delete_template", { file: t.file }).catch((e) => showError(page, e)); showTemplates(); } }, svg("trash", 16)));
    }
    group.append(row);
  }
  page.append(group);

  // New from headings: no TOML needed.
  const name = el("input", { type: "text", placeholder: "Client check-in", "aria-label": "Template name", maxlength: "60" });
  const rows = el("div", { class: "heading-rows" });
  const addRow = (text = "", kind = "list") => {
    const title = el("input", { type: "text", value: text, placeholder: "Heading", "aria-label": "Heading", maxlength: "60" });
    const type = el("select", { "aria-label": "Kind" }, ["paragraph", "list", "tasks"].map((k) =>
      el("option", { value: k, selected: k === kind }, { paragraph: "A paragraph", list: "A list of points", tasks: "Tasks: who, what, when" }[k])));
    const row = el("div", { class: "heading-row" }, title, type, el("button", { class: "icon-only", type: "button", "aria-label": "Remove", onclick: () => row.remove() }, svg("close", 14)));
    rows.append(row);
  };
  [["Decisions", "list"], ["Blockers", "list"], ["Client sentiment", "paragraph"], ["Follow-ups", "tasks"]].forEach(([t, k]) => addRow(t, k));
  const create = el("button", { class: "primary pill", onclick: async () => {
    const headings = [...rows.querySelectorAll(".heading-row")].map((r) => [r.querySelector("input").value.trim(), r.querySelector("select").value]).filter(([t]) => t);
    try {
      await invoke("new_template", { name: name.value.trim(), headings });
      showTemplates(`"${name.value.trim()}" is ready, and starred, so it's offered first under Analysis templates on every meeting and note.`);
    } catch (e) {
      showError(form, e);
    }
  } }, "Create template");
  const form = el("div", { class: "group template-new" },
    el("div", { class: "row" }, el("span", { class: "label" }, "Name"), name),
    el("div", { class: "row column" }, el("span", { class: "label" }, "Headings", el("small", {}, "Each becomes a section. Points and tasks link to the lines they come from.")), rows,
      el("button", { class: "link", type: "button", onclick: () => addRow() }, "+ Add heading")),
    el("div", { class: "row" }, el("span", { class: "grow" }), create));
  page.append(el("h2", {}, "New template"), form);
  if (isDesktop()) {
    page.append(el("p", { class: "hint" }, "Templates are TOML files in your Superteem Voice folder, so they can also be edited in any text editor. ",
      el("button", { class: "link inline", onclick: () => invoke("open_templates_folder") }, "Open the folder")));
  }
  main.replaceChildren(page);
}

async function editTemplate(t) {
  const { source, file } = await invoke("template_source", { id: t.file ?? t.id });
  leave();
  showPage();
  view = { kind: "template-edit" };
  const area = el("textarea", { class: "template-source mono", spellcheck: "false", "aria-label": "Template" }, source);
  const result = el("div", { role: "status" });
  const page = el("div", { class: "page" }, backLinks("‹ Analysis templates", () => showTemplates()),
    el("h1", {}, file ? t.name : `Customise ${t.name}`),
    el("p", { class: "hint" }, file ? "Changes take effect the next time it runs." : "Saving makes your own copy, which replaces the built-in until you delete it."),
    area, result,
    el("div", { class: "toolbar" }, el("span", { class: "grow" }),
      el("button", { class: "pill", onclick: () => showTemplates() }, "Cancel"),
      el("button", { class: "primary pill", onclick: async () => {
        try {
          const saved = await invoke("save_template", { source: area.value, file, was: t.id });
          showTemplates(`Saved. "${saved.name}" is used from now on, the next time it's made${analysisOrigin ? ", including with Make again" : ""}.`);
        } catch (e) {
          result.replaceChildren(el("pre", { class: "danger template-errors" }, String(e)));
        }
      } }, "Save")));
  main.replaceChildren(page);
  area.focus();
}

// ---------- Reading aloud

/// The one thing being read aloud: { button, cancelled }.
let reading = null;

/// A button that reads aloud what `text()` returns, and stops when pressed again.
/// `lang`: the language code what's read is in, or a function giving it; unset for the main voice.
/// `pieces()`, instead of text, gives the pieces to say already split (see speechPieces), and `onPiece` is
/// told each as it's said, then null when reading ends.
function readAloudButton(text, what = "Read aloud", { label, lang, pieces, onPiece } = {}) {
  const button = label
    ? el("button", { class: "pill with-icon read-aloud", type: "button", title: what, "aria-pressed": "false" }, svg("play", 14), label)
    : el("button", { class: "icon-only read-aloud", type: "button", title: what, "aria-label": what, "aria-pressed": "false" }, svg("speak", 16));
  // A labelled button says Stop while it reads.
  if (label) button.reading = (on) => button.replaceChildren(svg(on ? "pause" : "play", 14), on ? "Stop" : label);
  button.addEventListener("mousedown", (e) => e.preventDefault());
  button.addEventListener("click", () => {
    if (reading?.button === button) return stopReading();
    stopReading();
    readAloud(button, pieces ? null : text(), typeof lang === "function" ? lang() : lang, { pieces: pieces?.(), onPiece });
  });
  return button;
}

/// Marks a line of text to read aloud as a heading, so it has a pause before and after it.
const HEADING = "\u2063";

/// Pauses, in milliseconds at normal speed: after a sentence, after a paragraph (a line), and around a
/// heading.
const PAUSE = { phrase: 150, sentence: 350, paragraph: 700, heading: 800 };

/// The text as pieces to say, each with the pause before and after it: a heading on its own; otherwise a
/// sentence at a time, and a very long one at its commas.
function speechPieces(text) {
  const pieces = [];
  const lines = (text ?? "").replace(/\r/g, "").split("\n");
  for (let i = 0; i < lines.length; i++) {
    const raw = lines[i].trim();
    if (!raw) { if (pieces.length) pieces.at(-1).after = Math.max(pieces.at(-1).after, PAUSE.paragraph); continue; }
    // A line marked as one, or written as one in Markdown.
    if (raw.startsWith(HEADING) || /^#{1,6}\s/.test(raw)) {
      const heading = raw.replace(HEADING, "").replace(/^#{1,6}\s+/, "").trim();
      if (heading) pieces.push({ text: heading, before: PAUSE.heading, after: PAUSE.heading });
      continue;
    }
    // Sentences end at . ! ? and their Chinese forms, which have no space after them.
    const sentences = raw.split(/(?<=[.!?…])\s+|(?<=[。！？])/).map((s) => s.trim()).filter(Boolean);
    sentences.forEach((sentence, n) => {
      const last = n === sentences.length - 1;
      const parts = sentence.length > 280 ? sentence.split(/(?<=[,;，；])\s*/).filter(Boolean) : [sentence];
      parts.forEach((part, m) => {
        const end = m === parts.length - 1;
        pieces.push({ text: part, before: 0, after: !end ? PAUSE.phrase : last ? PAUSE.paragraph : PAUSE.sentence });
      });
    });
  }
  return pieces;
}

/// Waits `ms`, or less if reading is stopped.
function pause(ms, mine) {
  return new Promise((resolve) => {
    const started = Date.now();
    const tick = () => (mine.cancelled || Date.now() - started >= ms ? resolve() : setTimeout(tick, 30));
    tick();
  });
}

/// Says the text a sentence at a time, with a pause after each and longer ones around headings and between
/// paragraphs. Short pieces also keep long text going where a voice gives up after a while, and let it stop
/// between them.
async function readAloud(button, text, lang, { pieces: given, onPiece } = {}) {
  const pieces = given ?? speechPieces(text);
  if (!pieces.length) return;
  // The gap between two pieces: the longer of what the first wants after it and the next before it, shorter
  // when reading faster.
  const gap = (i) => (i + 1 < pieces.length ? Math.max(pieces[i].after, pieces[i + 1].before) : 0) / (prefs.read_speed ?? 1);
  const mine = { button, cancelled: false, onPiece };
  reading = mine;
  button.classList.add("on");
  button.setAttribute("aria-pressed", "true");
  button.title = "Stop reading";
  button.reading?.(true);
  // The listener doesn't take the app's own voice for someone talking.
  invoke("listen_mute", { muted: true }).catch(() => {});
  try {
    // The natural voice for this language, if there is one; otherwise the system's voice reads.
    const voice = isDesktop() ? await invoke("reading_voice", { language: lang ?? null }).catch(() => null) : null;
    if (voice) {
      // The natural voice makes the next piece while this one plays, so there's no gap between them.
      let next = invoke("natural_speak", { text: pieces[0].text, voice: voice.id });
      for (let i = 0; i < pieces.length && !mine.cancelled; i++) {
        const wav = await next;
        if (i + 1 < pieces.length) next = invoke("natural_speak", { text: pieces[i + 1].text, voice: voice.id });
        if (mine.cancelled) break;
        onPiece?.(pieces[i], voice);
        await playWav(wav, mine);
        await pause(gap(i), mine);
      }
    } else {
      for (let i = 0; i < pieces.length && !mine.cancelled; i++) {
        onPiece?.(pieces[i], null);
        await systemSay(pieces[i].text, lang);
        await pause(gap(i), mine);
      }
    }
  } catch (e) {
    showError(button.closest(".page") ?? main, e);
  }
  if (reading === mine) reading = null;
  // Unless reading started again from elsewhere in the same place, which now says where it is.
  if (reading?.onPiece !== onPiece) onPiece?.(null);
  button.classList.remove("on");
  button.setAttribute("aria-pressed", "false");
  button.title = "Read aloud";
  button.reading?.(false);
  if (!reading) invoke("listen_mute", { muted: false }).catch(() => {});
}

/// Says `text` in the system's voice chosen in Settings, at the chosen speed and pitch. (Named apart from
/// the voice panel's say() in app.js: two top-level functions with one name are one, the later.)
function systemSay(text, lang) {
  const tag = lang ? languageTag(lang) : "";
  if (platform === "android" || !window.speechSynthesis) return invoke("speak_text", { text, language: tag || null });
  return new Promise((resolve) => {
    const u = new SpeechSynthesisUtterance(text.replace(/[“”]/g, ""));
    u.rate = prefs.read_speed ?? 1;
    u.pitch = prefs.read_pitch ?? 1;
    // The voice chosen in Settings, unless the text is in a language it doesn't speak: then one that does.
    const all = speechSynthesis.getVoices();
    const speaks = (v) => !lang || v.lang.toLowerCase().startsWith(lang);
    const chosen = (prefs.system_voice && all.find((v) => v.name === prefs.system_voice && speaks(v))) || (lang && lang !== "en" ? all.find(speaks) : null);
    if (chosen) u.voice = chosen;
    if (tag) u.lang = tag;
    u.onend = u.onerror = () => resolve();
    speechSynthesis.speak(u);
  });
}

/// Plays a WAV the natural voice made; `mine.source` lets it be stopped.
async function playWav(wav, mine) {
  const audio = (playWav.context ||= new AudioContext());
  if (audio.state === "suspended") await audio.resume();
  const buffer = await audio.decodeAudioData(wav instanceof ArrayBuffer ? wav : new Uint8Array(wav).buffer);
  return new Promise((resolve) => {
    const source = audio.createBufferSource();
    source.buffer = buffer;
    source.connect(audio.destination);
    source.onended = resolve;
    mine.source = source;
    source.start();
  });
}

function stopReading() {
  if (!reading) return;
  try { reading.source?.stop(); } catch {}
  reading.cancelled = true;
  window.speechSynthesis?.cancel();
  if (platform === "android") invoke("stop_speaking").catch(() => {});
}

/// An analysis card as it reads aloud: headings and points, who and when said as words, without the
/// source chips and buttons.
function analysisSpeech(card) {
  const body = card.querySelector(".analysis-body");
  if (!body) return "";
  const lines = [HEADING + (card.querySelector(".analysis-name")?.textContent ?? "")];
  for (const node of body.querySelectorAll(".analysis-section > h3, .analysis-section > p, .analysis-points > li, .notice")) {
    if (node.tagName === "H3") { lines.push(HEADING + node.textContent); continue; }
    if (node.tagName !== "LI") { lines.push(node.textContent); continue; }
    const who = node.querySelector(".who-chip")?.textContent;
    const what = [...node.querySelectorAll(":scope > .editable, :scope > .field-extra .editable")].map((n) => n.textContent).join(", ");
    const when = node.querySelector(".when-chip:not(.empty)")?.textContent;
    const done = node.classList.contains("done") ? " Done." : "";
    const time = node.querySelector(".bookmark-time")?.textContent;
    const point = `${who && who !== "Unassigned" ? `${who}: ` : ""}${what}${when ? `, ${when}` : ""}`;
    lines.push(`${time ? `At ${time}. ` : ""}${point}${/[.!?。！？]$/.test(point) ? "" : "."}${done}`);
  }
  return lines.join("\n");
}

// ---------- Settings › Reading aloud

listen("tts-download", (e) => {
  document.querySelectorAll("[data-tts-progress]").forEach((n) => {
    if (n.dataset.ttsProgress === e.payload.id) n.textContent = `${Math.round(e.payload.fraction * 100)}%`;
  });
});

/// The system's voices, which some web views only list a moment after loading.
function systemVoices() {
  if (!window.speechSynthesis) return Promise.resolve([]);
  const now = speechSynthesis.getVoices();
  if (now.length) return Promise.resolve(now);
  return new Promise((resolve) => {
    const done = () => resolve(speechSynthesis.getVoices());
    speechSynthesis.addEventListener("voiceschanged", done, { once: true });
    setTimeout(done, 800);
  });
}

const speedText = (s) => (Math.abs(s - 1) < 0.01 ? "Normal" : `${s.toFixed(2).replace(/0$/, "")}×`);

/// One line for the Settings row: which voice, at what speed.
async function readingAloudSummary() {
  const status = await invoke("tts_status").catch(() => null);
  const natural = prefs.natural_voice && status?.voices.find((v) => v.id === status.chosen && v.ready);
  const speed = prefs.read_speed ?? 1;
  return `${natural ? `${natural.name}, natural voice` : prefs.system_voice || "The system's voice"}${Math.abs(speed - 1) < 0.01 ? "" : ` · ${speedText(speed)}`}`;
}

async function saveReading(change) {
  await invoke("save_voice_settings", change);
  Object.assign(prefs, {
    ...(change.natural != null && { natural_voice: change.natural }),
    ...(change.voice != null && { tts_voice: change.voice }),
    ...(change.speed != null && { read_speed: change.speed }),
    ...(change.pitch != null && { read_pitch: change.pitch }),
    ...(change.systemVoice != null && { system_voice: change.systemVoice }),
    ...(change.readSpeakers != null && { read_speakers: change.readSpeakers }),
  });
}

/// A slider from 0.5 to 2 with its value beside it, saved when let go.
function rateSlider(label, hint, value, save) {
  const shown = el("span", { class: "value mono" }, speedText(value));
  const input = el("input", { type: "range", min: "0.5", max: "2", step: "0.05", value: String(value), "aria-label": label });
  input.addEventListener("input", () => { shown.textContent = speedText(Number(input.value)); });
  input.addEventListener("change", () => save(Number(input.value)));
  const reset = el("button", { class: "link", type: "button", onclick: () => { input.value = "1"; shown.textContent = speedText(1); save(1); } }, "Reset");
  return el("div", { class: "row slider-row" }, el("span", { class: "label" }, label, hint ? el("small", {}, hint) : null),
    el("div", { class: "slider" }, input, shown, reset));
}

/// What a voice is, as its row says: "British English, female".
const voiceAbout = (v) => [v.language, v.gender?.toLowerCase()].filter(Boolean).join(", ");

/// Settings › Reading aloud: how it reads, then the voice, natural or the system's, and adding one.
async function showReadingSettings(message) {
  leave();
  showPage();
  view = { kind: "reading-settings" };
  refreshItems();
  const [status, voices] = await Promise.all([invoke("tts_status").catch(() => null), systemVoices()]);
  const page = el("div", { class: "page" }, backLinks("‹ Settings", () => showSettings()), el("h1", {}, "Reading aloud"));
  page.append(el("p", { class: "hint" }, "Notes, transcripts and analyses can be read aloud with the speaker button beside them. Choose how it reads and the voice here."));
  if (message) page.append(el("div", { class: "notice" }, message));
  const sample = "This is how notes, transcripts and summaries sound when they're read aloud.";
  const natural = !!prefs.natural_voice && !!status?.voices.some((v) => v.id === status.chosen && v.ready);
  const chosenVoice = natural ? status.voices.find((v) => v.id === status.chosen) : null;

  // How it reads.
  const how = el("div", { class: "group" },
    rateSlider("Speed", "How fast it reads", prefs.read_speed ?? 1, (s) => saveReading({ speed: s })));
  const systemNames = voices.filter((v) => /^en/i.test(v.lang)).map((v) => v.name);
  if (!natural) {
    how.append(rateSlider("Pitch", "The system's voice only", prefs.read_pitch ?? 1, (s) => saveReading({ pitch: s })));
    if (systemNames.length > 1) {
      const select = el("select", { "aria-label": "System voice" }, el("option", { value: "" }, "This device's default"),
        systemNames.map((n) => el("option", { value: n, selected: n === prefs.system_voice }, n)));
      select.addEventListener("change", () => saveReading({ systemVoice: select.value }));
      how.append(el("div", { class: "row stack-narrow" }, el("span", { class: "label" }, "System voice", el("small", {}, "The English voices this device has")), select));
    }
  }
  // A Chinese and English voice tries both.
  const trySample = chosenVoice?.langs?.includes("zh") ? `${sample} 这是用中文朗读的样子。` : sample;
  how.append(
    toggleRow("Say who's speaking", "Reading a transcript, say each speaker's name before what they said.", prefs.read_speakers ?? true,
      (on) => saveReading({ readSpeakers: on })),
    el("div", { class: "row" }, el("span", { class: "label" }, "Try it", el("small", {}, chosenVoice ? `${chosenVoice.name}, with the settings above` : "The system's voice, with the settings above")),
      readAloudButton(() => trySample, "Hear the voice", { label: "Play" })));
  page.append(el("h2", {}, "How it reads"), how);

  // The voice: the system's, or one of the natural ones, downloaded once.
  const group = el("div", { class: "group voice-list", role: "radiogroup", "aria-label": "Voice" });
  const pick = (on, title, about, choose, extra = []) => {
    const radio = el("input", { type: "radio", name: "reading-voice", checked: on, "aria-label": title });
    radio.addEventListener("change", choose);
    return el("label", { class: `row voice-row${on ? " chosen" : ""}` }, radio,
      el("span", { class: "label" }, title, on ? el("span", { class: "badge ok" }, "Reading") : null, el("small", {}, about)), ...extra);
  };
  group.append(pick(!natural, "The system's voice", status?.supported
    ? "Built into this device. Nothing to download."
    : "Built into this device. Natural voices are on Windows now, and coming here.",
  async () => { await saveReading({ natural: false }); showReadingSettings(); }));
  // Voices for the person's languages are listed; the rest, one or two for each other language, fold away.
  const mine = (await languageData()).chosen;
  const others = el("div", { class: "group voice-list", role: "radiogroup", "aria-label": "Voices for other languages" });
  for (const v of status?.supported ? status.voices : []) {
    const on = natural && status.chosen === v.id;
    const listed = on || v.ready || v.custom || status.downloading === v.model || v.langs?.some((l) => mine.includes(l));
    const size = v.size >= 1e9 ? `${(v.size / 1e9).toFixed(1)} GB` : `${Math.round(v.size / 1e6)} MB`;
    const extra = [];
    if (status.downloading && status.downloading === v.model) {
      extra.push(el("span", { class: "value" }, el("span", { class: "spinner" }), " ", el("span", { "data-tts-progress": v.id }, "Starting…")),
        el("button", { class: "pill", type: "button", onclick: (e) => { e.preventDefault(); invoke("cancel_tts_download"); } }, "Stop"));
    } else if (v.ready || v.custom) {
      extra.push(el("button", { class: "icon-only danger", type: "button", title: v.custom ? `Remove ${v.name}` : `Delete ${v.name}'s download`, "aria-label": `Delete ${v.name}`,
        onclick: async (e) => { e.preventDefault(); await invoke("delete_tts", { id: v.id }).catch((err) => showError(page, err)); await refreshPrefs(); showReadingSettings(); } }, svg("trash", 16)));
    } else {
      extra.push(el("button", { class: "pill", type: "button", disabled: !!status.downloading, onclick: (e) => { e.preventDefault(); getVoice(v); } }, `Download (${size})`));
    }
    const row = pick(on, `${v.name}${v.recommended ? " (recommended)" : ""}`, `${voiceAbout(v)}${v.custom ? " · Added by you" : ""}`, async () => {
      if (!v.ready) return getVoice(v);
      await saveReading({ voice: v.id, natural: true });
      showReadingSettings();
    }, extra);
    if (!v.ready) row.querySelector("input").disabled = !!status.downloading || v.custom;
    (listed ? group : others).append(row);
  }
  async function getVoice(v) {
    const run = invoke("download_tts", { id: v.id });
    setTimeout(() => showReadingSettings(), 150);
    try {
      await run;
      // A voice for another language reads that language only; the main voice stays as it is.
      const primary = languageInfo?.primary ?? "en";
      if (v.custom || !v.langs?.length || v.langs.includes(primary)) {
        await saveReading({ voice: v.id, natural: true });
        if (view.kind === "reading-settings") showReadingSettings(`${v.name} is ready, and reads from now on.`);
      } else if (view.kind === "reading-settings") {
        await refreshPrefs();
        showReadingSettings(`${v.name} is ready, and reads ${v.langs.map((l) => languageName(l)).join(" and ")} from now on.`);
      }
    } catch (e) {
      if (view.kind === "reading-settings") showReadingSettings(String(e));
    }
  }
  page.append(el("h2", {}, "Voice"), group);
  if (others.childElementCount) {
    page.append(el("details", { class: "more-voices" }, el("summary", {}, `Voices for other languages (${others.childElementCount})`),
      el("p", { class: "hint" }, "For reading translations and notes in those languages. A downloaded voice reads its language; your own language keeps the voice chosen above."), others));
  }
  const langs = await languageData();
  if (status?.supported && langs.chosen.length > 1) page.append(el("h2", {}, "Voice for each language"), voiceForLanguages(page, status, langs));
  if (status?.supported) {
    page.append(el("p", { class: "hint" }, "Natural voices run on this device, offline, once downloaded. Choose the system's voice to read without one."));
    page.append(el("h2", {}, "Add a voice"), addVoiceForm(page, !!status.downloading));
  }
  main.replaceChildren(page);
}

/// Adding a voice from sherpa-onnx's collection: its download address or its file, and what to call it.
function addVoiceForm(page, busy) {
  const COLLECTION = "https://k2-fsa.github.io/sherpa/onnx/tts/all/";
  const name = el("input", { type: "text", maxlength: "60", placeholder: "Mia", "aria-label": "Name" });
  const language = el("input", { type: "text", maxlength: "40", placeholder: "Australian English", list: "voice-languages", "aria-label": "Language" });
  const languages = el("datalist", { id: "voice-languages" },
    ["British English", "US English", "Australian English", "Chinese English", "Chinese (Mandarin)", "Arabic", "French", "Hindi", "Spanish", "Turkish", "Welsh"]
      .map((l) => el("option", { value: l })));
  const gender = el("select", { "aria-label": "Gender" }, el("option", { value: "" }, "Not given"), el("option", { value: "Female" }, "Female"), el("option", { value: "Male" }, "Male"));
  const url = el("input", { type: "url", placeholder: "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/….tar.bz2", "aria-label": "Model download address" });
  let file = "";
  const fileName = el("small", { class: "hint" });
  const choose = el("button", { class: "pill", type: "button" }, "Choose a file…");
  choose.addEventListener("click", async () => {
    const picked = await window.__TAURI__?.dialog?.open({ title: "Choose a voice model", filters: [{ name: "Voice model", extensions: ["bz2"] }] }).catch(() => null);
    if (picked) { file = picked; url.value = ""; fileName.textContent = picked.split(/[\\/]/).pop(); }
  });
  url.addEventListener("input", () => { if (url.value) { file = ""; fileName.textContent = ""; } });
  const speaker = el("input", { type: "number", min: "0", step: "1", value: "0", class: "mono", "aria-label": "Speaker number" });
  const status = el("div", { class: "hint", role: "status" });
  const add = el("button", { class: "primary pill", type: "button", disabled: busy }, "Add the voice");
  add.addEventListener("click", async () => {
    if (!name.value.trim()) return status.replaceChildren(el("span", { class: "danger" }, "Give the voice a name."));
    if (!url.value.trim() && !file) return status.replaceChildren(el("span", { class: "danger" }, "Paste the model's download address, or choose its file."));
    add.disabled = true;
    status.replaceChildren(el("span", { class: "spinner" }), " ", el("span", { "data-tts-progress": "new" }, "Starting…"), " Downloading and trying it…");
    try {
      const voice = await invoke("add_tts_voice", { voice: { name: name.value.trim(), language: language.value.trim(), gender: gender.value,
        speaker: Math.max(0, Math.floor(Number(speaker.value) || 0)), url: url.value.trim(), file } });
      await saveReading({ voice: voice.id, natural: true });
      showReadingSettings(`${voice.name} is added, and reads from now on.`);
    } catch (e) {
      add.disabled = false;
      status.replaceChildren(el("span", { class: "danger" }, String(e)));
    }
  });
  const field = (label, hint, ...input) => el("div", { class: "row stack-narrow" }, el("span", { class: "label" }, label, hint ? el("small", {}, hint) : null), ...input);
  const wide = (label, hint, input) => el("div", { class: "row column" }, el("span", { class: "label" }, label, hint ? el("small", {}, hint) : null), input);
  return el("div", { class: "group add-voice" },
    el("div", { class: "row" }, el("span", { class: "label" }, "From the sherpa-onnx collection",
      el("small", {}, "Listen to the voices there. On a model's page, copy its download address, or download its .tar.bz2 file. A model with several speakers lists their numbers.")),
    el("button", { class: "pill", type: "button", onclick: () => invoke("open_link", { url: COLLECTION }).catch(() => window.open(COLLECTION)) }, "Open the collection")),
    wide("Model", "Its download address, or its file", el("div", { class: "add-voice-model" }, url, el("span", { class: "or" }, "or"), choose, fileName)),
    field("Speaker", "0 for a model with one voice", speaker),
    field("Name", null, name),
    field("Language", "As you'd like it shown", language, languages),
    field("Gender", null, gender),
    el("div", { class: "row" }, el("span", { class: "label" }, status), add));
}

/// Which voice reads each of the person's languages: the main voice when it speaks it, the system's, or a
/// natural voice that does.
function voiceForLanguages(page, status, langs) {
  const group = el("div", { class: "group" });
  const name = (c) => langs.all.find(([code]) => code === c)?.[1] ?? c;
  const main = status.voices.find((v) => v.id === status.chosen);
  for (const code of langs.chosen) {
    const set = prefs.voice_for?.[code] ?? "";
    const speakers = status.voices.filter((v) => v.ready && v.langs?.includes(code));
    // Left to itself: the main voice when it speaks the language, else the first downloaded voice that does.
    const automatic = prefs.natural_voice ? (main?.ready && main.langs?.includes(code) ? main : speakers[0]) : null;
    const select = el("select", { "aria-label": `Voice for ${name(code)}` },
      el("option", { value: "" }, automatic ? `${automatic.name}${automatic === main ? " (the main voice)" : ""}` : "The system's voice"),
      automatic ? el("option", { value: "system", selected: set === "system" }, "The system's voice") : null,
      speakers.filter((v) => v !== automatic).map((v) => el("option", { value: v.id, selected: set === v.id }, `${v.name} · ${voiceAbout(v)}`)));
    select.addEventListener("change", async () => {
      try {
        await invoke("save_voice_settings", { voiceFor: [code, select.value] });
        prefs.voice_for = { ...(prefs.voice_for ?? {}) };
        if (select.value) prefs.voice_for[code] = select.value; else delete prefs.voice_for[code];
      } catch (e) { showError(page, e); }
    });
    const none = !speakers.length ? " No downloaded natural voice speaks it yet." : "";
    group.append(el("div", { class: "row stack-narrow" }, el("span", { class: "label" }, name(code),
      el("small", {}, `For notes and meetings in ${name(code)}.${none}`)), select));
  }
  return el("div", {}, group, el("p", { class: "hint" }, "A note or meeting is in your primary language unless you choose another for it, beside its status."));
}

async function refreshPrefs() {
  try { prefs = await invoke("get_prefs"); } catch {}
}
