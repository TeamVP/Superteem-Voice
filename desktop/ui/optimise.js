"use strict";

/// AI on this device: one page showing everything the models do here and how fast, the one-button flow
/// that optimises them for this device (languages, then the plan, then downloading and trying it), the
/// callout at the top of Settings, and the suggestion on the home page once a note and a meeting are made.

icon.spark = '<path d="M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8z"></path><path d="M19 15l.8 2.2L22 18l-2.2.8L19 21l-.8-2.2L16 18l2.2-.8z"></path>';

const bytes = (n) => (n >= 1e9 ? `${(n / 1e9).toFixed(1)} GB` : `${Math.max(1, Math.round(n / 1e6))} MB`);

/// The plan and the state of things, fetched fresh.
const optimiseData = () => invoke("optimise_plan");

/// Optimising tells the page how it's going.
let optimisePage = null;
listen("optimise-progress", (e) => {
  if (optimisePage?.isConnected) drawOptimiseProgress(optimisePage, e.payload);
});

// ---------- Settings: the callout and the row

/// For the top of Settings: a callout while optimising would change something, else nothing.
async function optimiseCallout() {
  const data = await optimiseData().catch(() => null);
  if (!data) return null;
  const todo = data.plan.steps.filter((s) => !s.done && !s.kind.startsWith("remove"));
  if (!data.progress.running && (!todo.length || (!data.stale && data.last))) return null;
  return el("div", { class: "optimise-callout" }, svg("spark", 22),
    el("div", { class: "grow" }, el("strong", {}, data.progress.running ? "Optimising Voice for this device…" : `Make Voice faster and smarter on this ${isPhone() ? "phone" : "computer"}`),
      el("small", {}, data.progress.running ? data.progress.step : optimiseSummary(todo))),
    el("button", { class: "primary pill", onclick: () => (data.progress.running ? showAiSettings() : showOptimise()) }, data.progress.running ? "See progress" : "Optimise"));
}

/// "A better speech model, the graphics processor and a Spanish voice."
function optimiseSummary(todo) {
  const parts = [];
  if (todo.some((s) => s.kind === "gpu")) parts.push("the graphics processor");
  if (todo.some((s) => s.kind === "speech")) parts.push("a better speech model");
  if (todo.some((s) => s.kind === "analysis")) parts.push("a better analysis model");
  const voices = todo.filter((s) => s.kind === "voice").map((s) => s.title.replace(/^Reading (.+?):.*/, "$1"));
  if (voices.length) parts.push(`${voices.length === 1 ? "a" : ""} natural ${voices.join(" and ")} voice${voices.length > 1 ? "s" : ""}`);
  if (!parts.length) return "Check what runs here and how fast.";
  const list = parts.length > 1 ? `${parts.slice(0, -1).join(", ")} and ${parts.at(-1)}` : parts[0];
  return `One click sets up ${list}, chosen for what this device runs well.`;
}

// ---------- The home page suggestion

/// Shown once a note and a meeting have been made, when optimising would change something. "Not now"
/// stops it; Settings still offers it.
async function optimiseNudge() {
  const data = await optimiseData().catch(() => null);
  if (!data?.suggest || data.progress.running) return null;
  const todo = data.plan.steps.filter((s) => !s.done && !s.kind.startsWith("remove"));
  const card = el("div", { class: "optimise-nudge", role: "region", "aria-label": "Optimise for this device" },
    svg("spark", 22),
    el("div", { class: "grow" }, el("strong", {}, `Voice can run better on this ${isPhone() ? "phone" : "computer"}`), el("small", {}, optimiseSummary(todo))),
    el("div", { class: "optimise-nudge-actions" },
      el("button", { class: "primary pill", onclick: () => showOptimise() }, "Optimise"),
      el("button", { class: "link", onclick: async () => { await invoke("optimise_dismiss").catch(() => {}); card.remove(); } }, "Not now")));
  return card;
}

// ---------- Settings › AI on this device

async function showAiSettings(message) {
  leave();
  showPage();
  view = { kind: "ai-settings" };
  refreshItems();
  const page = el("div", { class: "page ai-page" }, backLinks("‹ Settings", () => showSettings()), el("h1", {}, `AI on this ${isPhone() ? "phone" : "computer"}`));
  main.replaceChildren(page);
  const data = await optimiseData().catch((e) => { showError(page, e); return null; });
  if (!data) return;
  const { plan, current, last, progress } = data;
  const d = plan.profile;
  page.append(el("p", { class: "hint" }, "Everything Voice does with AI runs here, offline: turning speech into text, analyses and translations, and reading aloud. This is what each runs with, and how fast."));
  if (message) page.append(el("div", { class: "notice" }, message));

  if (progress.running) {
    optimisePage = el("div", { class: "group optimise-progress" });
    page.append(optimisePage);
    drawOptimiseProgress(optimisePage, progress);
  } else {
    const todo = plan.steps.filter((s) => !s.done && !s.kind.startsWith("remove"));
    page.append(el("div", { class: "optimise-callout" }, svg("spark", 22),
      el("div", { class: "grow" }, el("strong", {}, last ? (todo.length ? "Something here can be better" : "Optimised for this device") : "Not optimised yet"),
        el("small", {}, last ? `${todo.length ? optimiseSummary(todo) : "Nothing to change."} Last done ${timeText(last.at)}.` : optimiseSummary(todo))),
      el("button", { class: todo.length || !last ? "primary pill" : "pill", onclick: () => showOptimise() }, last ? "Optimise again" : "Optimise")));
  }

  // The device.
  const gpuText = d.gpu ? `${d.gpu}${d.gpu_memory_gb ? `, ${d.gpu_memory_gb.toFixed(0)} GB` : d.integrated ? ", shares the computer's memory" : ""}` : "None that models can use";
  page.append(el("h2", {}, "This device"), el("div", { class: "group" },
    infoRow("Graphics processor", d.gpu ? (current.gpu_on ? "Models run on it." : "Turned off: models run on the processor.") : plan.notes.find((n) => n.includes("graphics")) ?? "", gpuText),
    infoRow("Memory", "Decides how large a model runs well.", `${d.ram_gb.toFixed(0)} GB`),
    infoRow("Free space", "Where models are kept.", d.free < 1e15 ? bytes(d.free) : "—"),
    infoRow("Space models take", "Speech, analysis and voice models together.", bytes(current.storage))));

  // Each job.
  const where = (gpu) => (gpu ? "on the graphics processor" : "on the processor");
  const speechSpeed = last?.speech_s != null ? `10 s of speech in ${last.speech_s.toFixed(1)} s, ${where(last.speech_on_gpu)}` : "Speed is measured when you optimise.";
  const words = last?.words_per_s != null ? `About ${Math.round(last.words_per_s)} words a second, ${where(last.analysis_on_gpu)}` : "Speed is measured when you optimise.";
  page.append(el("h2", {}, "What runs here"), el("div", { class: "group" },
    infoRow("Speech to text", `${speechSpeed}. Dictation, commands and transcripts.`, current.speech_ready ? current.speech : `${current.speech} (not downloaded)`),
    infoRow("Analysis", current.server ? `On your server, ${current.server}.` : `${words}. Summaries, action items and templates.`, current.server ? "Server" : current.analysis ?? "None yet"),
    infoRow("Translation", "Made by the analysis model, line by line.", current.server ? "Server" : current.analysis ?? "None yet"),
    infoRow("Reading aloud", "Notes, transcripts, analyses and translations.", current.languages.map(([, v]) => v).filter(Boolean)[0] ?? "The system's voice")));

  // Each language.
  const quality = { good: "Transcribed well", fair: "Transcribed fairly well", weak: "Transcription is rough", none: "Can't be transcribed yet" };
  page.append(el("h2", {}, "Your languages"), el("div", { class: "group" }, current.languages.map(([code, voice, q, direct]) =>
    infoRow(languageName(code), `${quality[q] ?? ""} · ${direct ? "Analyses written in it" : "Analysed through English"}`, voice ?? "System voice"))));

  // Things to tidy.
  const removable = plan.steps.filter((s) => s.kind.startsWith("remove"));
  if (removable.length) {
    page.append(el("h2", {}, "Not needed any more"), el("div", { class: "group" }, removable.map((s) =>
      infoRow(s.title.replace(/^Remove /, ""), s.reason, bytes(s.frees)))),
    el("p", { class: "hint" }, "Optimising removes these, or remove them yourself on the pages below."));
  }
  if (plan.notes.length) page.append(el("ul", { class: "optimise-notes" }, plan.notes.map((n) => el("li", {}, n))));

  page.append(el("h2", {}, "Choose for yourself"), el("div", { class: "group" },
    linkRow("Analysis", "The model, the graphics processor and a local server", () => { analysisOrigin = null; showAnalysisSettings(); }),
    linkRow("Languages", "Your languages and the speech models", () => showLanguages()),
    linkRow("Reading aloud", "Voices for each language, speed", () => showReadingSettings())));
}

function infoRow(label, detail, value) {
  return el("div", { class: "row" }, el("span", { class: "label" }, label, detail ? el("small", {}, detail) : null), el("span", { class: "value" }, value ?? ""));
}

// ---------- The flow

/// Optimising: first the languages, then the plan to confirm, then it runs on the AI page.
async function showOptimise() {
  leave();
  showPage();
  view = { kind: "optimise" };
  refreshItems();
  const info = await languageData(true);
  const page = el("div", { class: "page optimise-page" }, backLinks("‹ AI on this device", () => showAiSettings()), el("h1", {}, "Optimise for this device"),
    el("p", { class: "hint" }, "First, the languages you speak and read in. Voice then picks the speech model, analysis model and voices that work best for them on this device."));
  const chosen = new Set(info.chosen);
  const list = el("div", { class: "group" });
  const draw = () => {
    list.replaceChildren(...[...chosen].map((code) => {
      const box = el("input", { type: "checkbox", checked: true, disabled: code === info.primary, "aria-label": languageName(code) });
      box.addEventListener("change", () => { chosen.delete(code); draw(); });
      return el("label", { class: "row" }, box, el("span", { class: "label" }, languageName(code), code === info.primary ? el("small", {}, "Your main language") : null));
    }));
    const add = el("select", { "aria-label": "Add a language" }, el("option", { value: "" }, "Add a language…"),
      (info.catalogue ?? []).filter((l) => !chosen.has(l.code)).map((l) => el("option", { value: l.code }, l.native && l.native !== l.name ? `${l.name} · ${l.native}` : l.name)));
    add.addEventListener("change", () => { if (add.value) { chosen.add(add.value); draw(); } });
    list.append(el("div", { class: "row" }, el("span", { class: "label" }, "Another language"), add));
  };
  draw();
  const next = el("button", { class: "primary pill" }, "Next: see the plan");
  next.addEventListener("click", async () => {
    next.disabled = true;
    try {
      await invoke("save_languages", { languages: [...chosen], primary: info.primary });
      await languageData(true);
      showOptimisePlan();
    } catch (e) {
      next.disabled = false;
      showError(page, e);
    }
  });
  page.append(el("h2", {}, "Your languages"), list, el("div", { class: "toolbar" }, el("span", { class: "grow" }), next));
  main.replaceChildren(page);
}

async function showOptimisePlan() {
  leave();
  showPage();
  view = { kind: "optimise" };
  const page = el("div", { class: "page optimise-page" }, backLinks("‹ Languages", () => showOptimise()), el("h1", {}, "The plan for this device"));
  page.append(el("p", { class: "hint" }, el("span", { class: "spinner" }), " Looking at this device…"));
  main.replaceChildren(page);
  const data = await optimiseData().catch((e) => { showError(page, e); return null; });
  if (!data) return;
  page.querySelector(".hint").remove();
  const { plan } = data;
  const d = plan.profile;
  page.append(el("p", { class: "hint" }, `${d.gpu ?? "No graphics processor models can use"} · ${d.ram_gb.toFixed(0)} GB of memory · ${d.free < 1e15 ? bytes(d.free) : "plenty of"} free. Untick anything you'd rather not have.`));
  const picked = new Map();
  const total = el("strong", {});
  const go = el("button", { class: "primary pill" });
  const sum = () => {
    const steps = plan.steps.filter((s) => picked.get(`${s.kind}:${s.id}`));
    const download = steps.reduce((n, s) => n + s.download, 0);
    const frees = steps.reduce((n, s) => n + s.frees, 0);
    total.textContent = download ? `Downloads ${bytes(download)}${frees ? `, frees ${bytes(frees)}` : ""}` : frees ? `Frees ${bytes(frees)}` : steps.length ? "Nothing to download" : "Nothing to change";
    go.textContent = download ? `Download and set up (${bytes(download)})` : "Set up";
    go.disabled = false;
  };
  const group = el("div", { class: "group optimise-plan" });
  for (const s of plan.steps) {
    const key = `${s.kind}:${s.id}`;
    picked.set(key, !s.done);
    const box = el("input", { type: "checkbox", checked: !s.done, disabled: s.done, "aria-label": s.title });
    box.addEventListener("change", () => { picked.set(key, box.checked); sum(); });
    const size = s.done ? el("span", { class: "badge ok" }, svg("check", 12), "Set up") : el("span", { class: "value" }, s.download ? bytes(s.download) : s.frees ? `Frees ${bytes(s.frees)}` : "");
    group.append(el("label", { class: `row${s.done ? " done" : ""}` }, box, el("span", { class: "label" }, s.title, el("small", {}, s.reason)), size));
  }
  page.append(group);
  if (plan.notes.length) page.append(el("ul", { class: "optimise-notes" }, plan.notes.map((n) => el("li", {}, n))));
  page.append(el("p", { class: "hint" }, "Each model is tried once when it's in place, so you see how fast it runs here. Downloads carry on if you leave this page."));
  go.addEventListener("click", async () => {
    const steps = plan.steps.filter((s) => picked.get(`${s.kind}:${s.id}`)).map((s) => [s.kind, s.id]);
    go.disabled = true;
    try {
      await invoke("optimise_apply", { steps });
      showAiSettings();
    } catch (e) {
      go.disabled = false;
      showError(page, e);
    }
  });
  page.append(el("div", { class: "toolbar optimise-go" }, total, el("span", { class: "grow" }), el("button", { class: "pill", onclick: () => showAiSettings() }, "Cancel"), go));
  sum();
  main.replaceChildren(page);
}

function drawOptimiseProgress(holder, p) {
  if (!p.running) {
    if (p.finished) {
      const problems = p.problems ?? [];
      const message = p.error ?? (problems.length ? `Optimised, except: ${problems.join(" · ")}` : "Optimised. Everything below is set up and was tried on this device.");
      optimisePage = null;
      if (view.kind === "ai-settings") showAiSettings(message);
    }
    return;
  }
  const bar = el("div", { class: "optimise-bar" }, el("span", { style: `width:${Math.round((p.fraction ?? 0) * 100)}%` }));
  holder.replaceChildren(el("div", { class: "row" }, el("span", { class: "spinner" }),
    el("span", { class: "label" }, p.step || "Starting…", el("small", {}, `Step ${p.index} of ${p.count}${p.fraction > 0 && p.fraction < 1 ? ` · ${Math.round(p.fraction * 100)}%` : ""}`), bar),
    el("button", { class: "pill", onclick: () => invoke("optimise_cancel") }, "Stop")));
}
