"use strict";

/// Analyses: a stack of template results at the top of a meeting or note, under the title. Each card can be
/// collapsed, made again or removed; action items are a checklist whose owners can be reassigned; every
/// claim has source chips that show the line it came from and go there. Also the Settings pages for the
/// language model and for templates.

icon.more = '<circle cx="5" cy="12" r="1.5" fill="currentColor" stroke="none"></circle><circle cx="12" cy="12" r="1.5" fill="currentColor" stroke="none"></circle><circle cx="19" cy="12" r="1.5" fill="currentColor" stroke="none"></circle>';
icon.sparkle = '<path d="M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8z"></path><path d="M19 15l.8 2.2L22 18l-2.2.8L19 21l-.8-2.2L16 18l2.2-.8z"></path>';

/// The page showing analyses now: { kind, id, holder, ctx }.
let analysisPage = null;
/// "Reading part 2 of 3…", by job.
const analysisProgress = new Map();
const jobKey = (j) => `${j.kind}/${j.id}/${j.template}`;

listen("analysis-queue", () => refreshAnalyses());
listen("analysing", (e) => {
  analysisProgress.set(jobKey(e.payload), e.payload.parts > 1 ? `Reading part ${e.payload.part} of ${e.payload.parts}…` : "Reading…");
  refreshAnalyses();
});
listen("analysed", (e) => {
  analysisProgress.delete(jobKey(e.payload));
  if (e.payload.error && analysisPage && analysisPage.kind === e.payload.kind && analysisPage.id === e.payload.id) {
    analysisPage.errors.set(e.payload.template, e.payload.error);
  }
  refreshAnalyses();
});

/// The stack for a meeting or note. `ctx`: { speakers: [names], go(source) }.
function analysisStack(kind, id, ctx) {
  const holder = el("section", { class: "analyses", "aria-label": "Analyses" });
  analysisPage = { kind, id, holder, ctx, errors: new Map() };
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
  const cards = data.entries.map((e) => analysisCard(page, e, data, running(e.template.id), waiting(e.template.id)));
  // A template asked for that has no result yet shows as a card in progress.
  const pending = data.templates.filter((t) => !data.entries.some((e) => e.template.id === t.id) && (running(t.id) || waiting(t.id)));
  for (const t of pending) cards.push(pendingCard(page, t, running(t.id)));
  for (const [template, error] of page.errors) {
    if (data.entries.some((e) => e.template.id === template) || pending.some((t) => t.id === template)) continue;
    const name = data.templates.find((t) => t.id === template)?.name ?? template;
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

function addAnalysis(page, data) {
  const used = new Set(data.entries.map((e) => e.template.id));
  const busy = (t) => data.queue.waiting.some((j) => isJob(j, page, t)) || isJob(data.queue.current, page, t);
  const choices = data.templates.filter((t) => !used.has(t.id) && !busy(t.id));
  const bar = el("div", { class: "analysis-add" });
  if (!data.ready) {
    bar.append(el("button", { class: "link with-icon", onclick: () => showAnalysisSettings() }, svg("sparkle", 16),
      data.entries.length ? "Get a language model to make these again" : "Summaries and action items: get a language model"));
    return bar;
  }
  if (!choices.length) return bar;
  bar.append(menuButton(el("span", { class: "with-icon" }, svg("sparkle", 16), data.entries.length ? "Add another analysis" : "Summarise"),
    choices.map((t) => [t.name, t.description, () => runAnalysis(page, t.id)]), "link add-analysis"));
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
  for (const [text, detail, run, disabled] of choices) {
    const item = el("button", { type: "button", role: "menuitem", disabled: !!disabled }, el("strong", {}, text), detail ? el("small", {}, detail) : null);
    item.addEventListener("click", () => { close(); run(); });
    menu.append(item);
  }
  wrap.append(button, menu);
  return wrap;
}

function pendingCard(page, t, running) {
  const progress = analysisProgress.get(jobKey({ kind: page.kind, id: page.id, template: t.id }));
  return el("div", { class: "analysis-card pending" },
    el("div", { class: "analysis-head" }, el("span", { class: "spinner" }), el("strong", {}, t.name),
      el("span", { class: "hint" }, running ? progress ?? "Loading the model…" : waitingText()),
      el("span", { class: "grow" }),
      el("button", { class: "link", onclick: () => invoke("cancel_analysis", { kind: page.kind, id: page.id, template: t.id }) }, "Cancel")));
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
  const template = e.template.id;
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
  const more = menuButton(svg("more", 16), [
    ["Copy as text", null, () => navigator.clipboard?.writeText(card.querySelector(".analysis-body")?.innerText ?? "")],
    ["Move up", null, () => invoke("move_analysis", { kind: page.kind, id: page.id, template, by: -1 }).then(refreshAnalyses), index === 0],
    ["Move down", null, () => invoke("move_analysis", { kind: page.kind, id: page.id, template, by: 1 }).then(refreshAnalyses), index === data.entries.length - 1],
    ["Remove", "Edits and ticks here are lost", remove],
  ]);
  more.querySelector("button").setAttribute("aria-label", "More");
  const head = el("div", { class: "analysis-head" }, toggle,
    el("button", { class: "analysis-name", onclick: () => edit({ op: "collapse", collapsed: !e.collapsed }) }, e.template.name),
    e.collapsed && def ? el("span", { class: "digest" }, analysisDigest(def, e.result)) : null,
    ...badges, el("span", { class: "grow" }), regenerate, more);
  card.append(head);
  if (error) card.append(el("p", { class: "danger analysis-error" }, `Couldn't make it again: ${error}`));
  if (e.collapsed) return card;
  if (!def) {
    card.append(el("p", { class: "hint" }, "Its template is no longer available, so it can only be removed."));
    return card;
  }
  const body = el("div", { class: "analysis-body" });
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
  const made = e.engine.kind === "local" ? "this device" : e.engine.host ?? "a server";
  const footer = el("div", { class: "analysis-foot" },
    `Made on ${made} with ${e.engine.model} · ${dateText(e.created)} · Check it against the ${page.kind === "note" ? "note" : "transcript"}.`);
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

async function showAnalysisSettings(message) {
  leave();
  showPage();
  view = { kind: "analysis-settings" };
  refreshItems();
  const status = await invoke("llm_status");
  const page = el("div", { class: "page" }, el("button", { class: "link back-link", onclick: () => showSettings() }, "‹ Settings"),
    el("h1", {}, "Analysis"));
  page.append(el("p", { class: "hint" }, "Summaries, decisions, action items and other notes are made from transcripts by a small language model that runs on this device. ",
    "Download one once; after that it works offline and nothing is sent anywhere."));
  if (message) page.append(el("div", { class: "notice" }, message));
  const { device } = status;
  page.append(el("p", { class: "hint" }, `This ${platform === "android" ? "phone" : "computer"} has ${device.ram_gb.toFixed(0)} GB of memory, so models read up to ${device.context.toLocaleString()} tokens at a time${device.too_small ? ". That's too little to run a model well here; a local server (below) can do it instead." : "."}`));

  const installed = new Map(status.installed.map((i) => [i.file, i]));
  const group = el("div", { class: "group" });
  for (const m of status.catalogue) {
    const have = installed.has(m.file);
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
  if (platform === "windows") {
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
    linkRow("Templates", "The kinds of notes that can be made, and your own", () => showTemplates())));

  // A server the person runs: never used unless set here, and anything not on this computer is named first.
  const url = el("input", { type: "url", value: status.server, placeholder: "http://localhost:11434/v1", "aria-label": "Server address" });
  const model = el("input", { type: "text", value: status.server_model, placeholder: "qwen2.5:3b", "aria-label": "Model name on the server" });
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

// ---------- Settings › Templates

async function showTemplates(message) {
  leave();
  showPage();
  view = { kind: "templates" };
  refreshItems();
  const all = await invoke("list_templates");
  const page = el("div", { class: "page" }, el("button", { class: "link back-link", onclick: () => showAnalysisSettings() }, "‹ Analysis"),
    el("h1", {}, "Templates"));
  page.append(el("p", { class: "hint" }, "A template says what to pull out of a meeting or note and how to lay it out. Run any of them from the top of a meeting or note; each one stacks there and can be made again."));
  if (message) page.append(el("div", { class: "notice" }, message));
  const group = el("div", { class: "group" });
  for (const t of all) {
    const kinds = t.applies_to.length === 2 ? "Meetings and notes" : t.applies_to[0] === "note" ? "Notes" : "Meetings";
    const label = el("span", { class: "label" }, t.name,
      t.source !== "built-in" ? el("span", { class: "badge" }, t.source === "customised" ? "Customised" : "Yours") : null,
      el("small", {}, t.errors.length ? "" : `${t.description} · ${kinds}${t.manual_only ? " · Only when asked" : ""}`));
    if (t.errors.length) label.append(el("ul", { class: "danger template-errors" }, t.errors.map((e) => el("li", {}, e))));
    const row = el("div", { class: "row" }, label);
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
      showTemplates(`"${name.value.trim()}" is ready to use.`);
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
  if (platform === "windows") {
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
  const page = el("div", { class: "page" }, el("button", { class: "link back-link", onclick: () => showTemplates() }, "‹ Templates"),
    el("h1", {}, file ? t.name : `Customise ${t.name}`),
    el("p", { class: "hint" }, file ? "Changes take effect the next time it runs." : "Saving makes your own copy, which replaces the built-in until you delete it."),
    area, result,
    el("div", { class: "toolbar" }, el("span", { class: "grow" }),
      el("button", { class: "pill", onclick: () => showTemplates() }, "Cancel"),
      el("button", { class: "primary pill", onclick: async () => {
        try {
          await invoke("save_template", { source: area.value, file });
          showTemplates("Saved.");
        } catch (e) {
          result.replaceChildren(el("pre", { class: "danger template-errors" }, String(e)));
        }
      } }, "Save")));
  main.replaceChildren(page);
  area.focus();
}
