// The vault: getting back into it, the recovery words, the vault password, the sealed copies, and bringing
// notes, books and meetings in from a backup or another computer.

let vault = null; // what `vault_status` last said

async function refreshVault() {
  vault = await invoke("vault_status").catch(() => null);
  return vault;
}

const vaultOpen = () => !vault || vault.state === "open";
/// "1 note", "3 notes": the keys "{n} note" and "{n} notes" are translated as one count's forms.
const vaultPlural = (n, word, many = `${word}s`) => trn(n, `{n} ${word}`, `{n} ${many}`);

/// Runs `open` when the vault is open; otherwise shows the way back in.
function vaultGate(open) {
  return () => (vaultOpen() ? open() : vault.state === "damaged" ? showVaultCopies() : showVaultUnlock());
}

function vaultDate(unix) {
  return new Date(unix * 1000).toLocaleDateString(UI_LOCALE ?? "en-GB", { day: "numeric", month: "short", year: "numeric" });
}

function vaultWhen(unix) {
  return new Date(unix * 1000).toLocaleString(UI_LOCALE ?? "en-GB", { day: "numeric", month: "short", year: "numeric", hour: "2-digit", minute: "2-digit" });
}

function vaultSize(bytes) {
  return bytes < 1024 * 1024 ? `${Math.max(1, Math.round(bytes / 1024))} KB` : `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

/// "7F3A90C2" as people see it: "7F3A-90C2".
const vaultId = (id) => (id && id.length === 8 && !id.includes("-") ? `${id.slice(0, 4)}-${id.slice(4)}` : id ?? "");

/// Appends what's given, leaving out the parts that are null (as `el` does for its children).
const vaultAdd = (parent, ...kids) => parent.append(...kids.flat().filter((k) => k != null && k !== false));

/// A sheet with a heading, built by `fill(dialog, body)`. Closing it removes it.
function vaultSheet(label, fill, { wide = false } = {}) {
  const body = el("div", { class: "vault-sheet-body" });
  const dialog = el("dialog", { class: "sheet vault-sheet" + (wide ? " wide" : ""), "aria-label": label }, el("h2", {}, label), body);
  dialog.addEventListener("close", () => dialog.remove());
  document.body.append(dialog);
  fill(dialog, body);
  dialog.showModal();
  return dialog;
}

/// Runs what a button does with the button off and its problem, if any, shown in `into`.
async function vaultTry(button, into, work) {
  into.querySelector(":scope > .error")?.remove();
  button.disabled = true;
  try {
    return await work();
  } catch (e) {
    into.append(el("div", { class: "error", role: "alert" }, String(e)));
    return undefined;
  } finally {
    button.disabled = false;
  }
}

/// After the vault opens or changes: the list, and whichever page is showing, are read again.
async function vaultChanged() {
  await refreshVault();
  refreshItems();
  if (view.kind === "vault") showVaultSettings();
  else if (view.kind === "home") showHome();
}

// ---------- Home: the way back in, what was moved in, and the reminder to save the words ----------

/// What the home page shows about the vault, or null when there's nothing to say.
function vaultCard() {
  if (!vault) return null;
  const card = (title, text, ...actions) => el("div", { class: "optimise-nudge vault-card", role: "region", "aria-label": tr("Vault") },
    svg("lock", 22), el("div", { class: "grow" }, el("strong", {}, title), el("small", {}, text)), el("div", { class: "optimise-nudge-actions" }, ...actions));
  if (vault.state === "locked") {
    return card(tr("Your notes, books and meetings are locked"), tr("This computer can't open your vault alone. Your recovery words or vault password open it."),
      el("button", { class: "primary pill", onclick: () => showVaultUnlock() }, tr("Open")));
  }
  if (vault.state === "damaged") {
    return card(tr("Your vault needs restoring"), tr("The vault can't be read, so notes, books and meetings are locked. An earlier copy can replace it."),
      el("button", { class: "primary pill", onclick: () => showVaultCopies() }, tr("Restore a copy")));
  }
  // A new computer whose synced Documents holds the vault from the old one.
  const other = vault.others?.[0];
  if (vault.sealed && other && vault.own_notes === 0) {
    return card(tr("Your vault from another computer is here"), tr("A copy from {0} (vault {1}) is in your backups. Its recovery words open it here.", { "0": vaultDate(other.at), "1": vaultId(other.id) }),
      el("button", { class: "primary pill", onclick: () => showVaultBringIn({ path: vault.backup_dir, folder: true, adopt: true }) }, tr("Open it here")));
  }
  if (vault.moved && vault.sealed) {
    const later = el("button", { class: "link" }, tr("Got it"));
    const made = card(tr("Your notes are now in an encrypted vault"), movedText(vault.moved),
      el("button", { class: "primary pill", onclick: () => showVaultWelcome() }, tr("What this means")), later);
    later.addEventListener("click", async () => { await invoke("vault_moved_seen").catch(() => {}); made.remove(); refreshVault(); });
    return made;
  }
  if (vault.remind) {
    const later = el("button", { class: "link" }, tr("Not now"));
    const made = card(tr("Save your recovery words"), tr("Your notes, books and meetings are encrypted and only this computer opens them alone. 24 words get you back in if it ever can't."),
      el("button", { class: "primary pill", onclick: () => showVaultWords() }, tr("Show words")), later);
    later.addEventListener("click", async () => { await invoke("vault_remind_later").catch(() => {}); made.remove(); refreshVault(); });
    return made;
  }
  return null;
}

function movedText(moved) {
  const parts = [moved.notes ? vaultPlural(moved.notes, "note") : null, moved.books ? vaultPlural(moved.books, "book") : null].filter(Boolean);
  const what = parts.length ? tr("{0} moved in", { "0": trList(parts) }) : tr("Your notes were moved in");
  const left = vault?.sealing ? tr(" Meetings are being sealed in the background: {0} to go.", { "0": vaultPlural(vault.sealing, "file") }) : tr(" Meetings are sealed too.");
  return `${what}.${left}`;
}

/// Once, after this version moves the person's notes, books and meetings into the vault: what changed,
/// and the one thing worth doing now.
function showVaultWelcome() {
  vaultSheet(tr("Your notes are now in an encrypted vault"), (dialog, body) => {
    const moved = vault?.moved ?? { notes: 0, books: 0, failed: [] };
    const seen = () => invoke("vault_moved_seen").catch(() => {});
    dialog.addEventListener("close", seen);
    vaultAdd(body, 
      el("p", { class: "hint" }, tr("{moved} Everything was read back and checked before the old files were removed.", { moved: movedText(moved) })),
      el("ul", { class: "vault-list" },
        el("li", {}, el("strong", {}, tr("Nothing changes day to day. ")), tr("Voice-OS opens the vault itself for your Windows account; no password to type.")),
        el("li", {}, el("strong", {}, tr("Copies can't be read. ")), tr("Backups, synced folders and copied disks hold only sealed files.")),
        el("li", {}, el("strong", {}, tr("A sealed copy is kept every day ")), `in ${vault?.backup_dir ?? tr("Documents › Voice-OS › Vault backups")}.`),
        el("li", {}, el("strong", {}, tr("Meetings open only in Voice-OS. ")), tr("Export makes readable copies when you need them elsewhere."))),
      moved.failed?.length ? el("div", { class: "notice" }, tr("Not moved, left in place: {0}.", { "0": moved.failed.join(", ") })) : null,
      el("div", { class: "notice" }, el("strong", {}, tr("One thing to do now: save your recovery words. ")),
        tr("If Windows is reinstalled or this computer replaced, they get you back in. Nobody else, Superteem included, can open your vault.")),
      el("div", { class: "actions" }, el("button", { class: "pill", onclick: () => dialog.close() }, tr("Later")),
        el("button", { class: "primary pill", onclick: () => { dialog.close(); showVaultWords(); } }, tr("Save my recovery words"))));
  });
}

/// The line in the sidebar when nothing can be listed.
function vaultItem() {
  if (vaultOpen()) return null;
  const damaged = vault.state === "damaged";
  return el("button", { class: "item vault-item", onclick: () => (damaged ? showVaultCopies() : showVaultUnlock()) },
    el("span", { class: "icon" }, svg("lock", 16)),
    el("span", { class: "body" }, el("span", { class: "title" }, damaged ? tr("Vault needs restoring") : tr("Notes, books and meetings are locked")),
      el("span", { class: "sub" }, damaged ? tr("Restore a copy") : tr("Open with recovery words"))));
}

// ---------- Getting back in ----------

/// A box for 24 recovery words that says how many are in it, and which vault they open, as they're typed.
function vaultWordsBox(placeholder) {
  const box = el("textarea", { rows: "5", spellcheck: "false", autocapitalize: "off", autocomplete: "off", "aria-label": tr("Recovery words"), placeholder });
  const count = el("small", { class: "vault-count", "aria-live": "polite" }, "0 of 24 words");
  let asked = 0;
  box.addEventListener("input", async () => {
    const n = box.value.split(/[^\p{L}]+/u).filter(Boolean).length;
    count.textContent = `${n} of 24 words`;
    count.classList.toggle("ok", false);
    count.classList.remove("bad");
    if (n !== 24) return;
    const mine = ++asked;
    try {
      const id = await invoke("vault_words_id", { words: box.value });
      if (mine !== asked) return;
      count.textContent = tr("24 words · for vault {id}", { id: id });
      count.classList.add("ok");
    } catch (e) {
      if (mine !== asked) return;
      count.textContent = String(e);
      count.classList.add("bad");
    }
  });
  return [box, count];
}

function showVaultUnlock() {
  vaultSheet(tr("Open your notes, books and meetings"), (dialog, body) => {
    const [box, count] = vaultWordsBox(tr("Type or paste your 24 recovery words"));
    const open = el("button", { class: "primary pill" }, tr("Open"));
    const done = async (status) => {
      if (!status) return;
      vault = status;
      dialog.close();
      await vaultChanged();
      if (vault.state === "damaged") showVaultCopies();
      else micToast(tr("Your vault is open, and this computer opens it itself from now on."));
    };
    open.addEventListener("click", async () => done(await vaultTry(open, body, () => invoke("vault_unlock_words", { words: box.value }))));
    vaultAdd(body, 
      el("p", { class: "hint" }, tr("This computer can't open your vault itself. That happens after Windows is reinstalled, your Windows password is reset by someone else, or the vault came from another computer. Nothing is lost: your notes, books and meetings are still here, encrypted.")),
      el("label", { class: "field" }, el("span", {}, tr("Recovery words"), el("small", {}, tr("The 24 words from your recovery kit. Case and spacing don't matter; the first four letters of each word are enough."))), box, count),
      el("div", { class: "actions" }, open));
    if (vault?.password) {
      const password = el("input", { type: "password", autocomplete: "off", "aria-label": tr("Vault password") });
      const use = el("button", { class: "pill" }, tr("Open with password"));
      use.addEventListener("click", async () => done(await vaultTry(use, body, () => invoke("vault_unlock_password", { password: password.value }))));
      password.addEventListener("keydown", (e) => { if (e.key === "Enter") use.click(); });
      vaultAdd(body, el("div", { class: "vault-or" }, "or"),
        el("label", { class: "field" }, el("span", {}, tr("Vault password"), vault.hint ? el("small", {}, tr("Your hint: {hint}", { hint: vault.hint })) : null), password),
        el("div", { class: "actions" }, use));
    }
    const fresh = el("button", { class: "pill danger" }, tr("Start an empty vault"));
    fresh.addEventListener("click", async () => {
      if (fresh.dataset.sure !== "yes") {
        fresh.dataset.sure = "yes";
        fresh.textContent = tr("Yes, start an empty vault");
        return;
      }
      const status = await vaultTry(fresh, body, () => invoke("vault_start_new"));
      if (!status) return;
      vault = status;
      dialog.close();
      await vaultChanged();
      micToast(tr("A new vault is open. The old one was set aside, not deleted."));
    });
    vaultAdd(body, el("details", { class: "vault-more" }, el("summary", {}, tr("I don't have the words")),
      el("p", { class: "hint" }, tr("Look for a file or printout named “Voice-OS recovery kit”, a password manager entry, or a photo of 24 numbered words. The kit names the vault ID its words are for. "),
        tr("If this vault was on another computer that still works, open Voice-OS there: Settings › Vault shows the words again.")),
      el("p", { class: "hint" }, tr("Without the words or password, nobody can open this vault, Superteem included. You can start an empty one and keep using Voice-OS. "),
        tr("The locked vault is set aside untouched, and its meetings stay where they are, shown as locked. Both open again if the words turn up: Settings › Vault › Bring in from a backup.")),
      el("div", { class: "actions" }, fresh)),
      el("div", { class: "actions" }, el("button", { class: "pill", onclick: () => dialog.close() }, tr("Close"))));
    box.focus();
  });
}

/// The sealed copies on this computer, to bring one back in place of the vault.
function showVaultCopies() {
  vaultSheet(vault?.state === "damaged" ? tr("Restore your vault") : tr("Restore a copy"), (dialog, body) => {
    const damaged = vault?.state === "damaged";
    vaultAdd(body, el("p", { class: "hint" }, damaged
      ? tr("The vault can't be read{0}. Voice-OS keeps sealed copies as it changes; choose one to replace it. The unreadable vault is set aside, not deleted.", { "0": vault.detail ? ` (${vault.detail.replace(/\.$/, "")})` : "" })
      : tr("Voice-OS keeps sealed copies of the vault as it changes. Restoring one puts every note and book back as it was. The vault as it is now is copied first, so this can be undone.")));
    if (!vault?.snapshots.length) {
      vaultAdd(body, el("p", { class: "notice" }, tr("No copies on this computer yet.")),
        damaged ? el("p", { class: "hint" }, tr("If Documents is backed up or synced, a daily copy is in Documents › Voice-OS › Vault backups. Open the vault, or start an empty one, then use Settings › Vault › Bring in from a backup.")) : null);
    }
    const list = el("div", { class: "group vault-copies" });
    for (const copy of vault?.snapshots ?? []) {
      const back = el("button", { class: "pill" }, tr("Restore"));
      back.addEventListener("click", async () => {
        if (back.dataset.sure !== "yes" && !damaged) {
          back.dataset.sure = "yes";
          back.textContent = tr("Yes, restore it");
          return;
        }
        const status = await vaultTry(back, body, () => invoke("vault_restore", { file: copy.file }));
        if (!status) return;
        vault = status;
        dialog.close();
        await vaultChanged();
        micToast(tr("Your notes are as they were on {0}.", { "0": vaultWhen(copy.at) }));
      });
      list.append(el("div", { class: "row" }, el("span", { class: "label" }, vaultWhen(copy.at), el("small", {}, vaultSize(copy.bytes))), back));
    }
    if (list.children.length) vaultAdd(body, list);
    if (damaged && !vault.key_known) vaultAdd(body, el("p", { class: "hint" }, tr("Open the vault with your recovery words first, then restore a copy.")));
    vaultAdd(body, el("div", { class: "actions" },
      damaged ? el("button", { class: "pill", onclick: () => { dialog.close(); showVaultUnlock(); } }, tr("Other ways in")) : null,
      el("button", { class: "pill", onclick: () => dialog.close() }, tr("Close"))));
  });
}

// ---------- The recovery words ----------

/// Prints `text` on its own, in a fixed-width face, without the rest of the page.
function vaultPrint(text) {
  const sheet = el("pre", { class: "vault-print" }, text);
  document.body.append(sheet);
  document.body.classList.add("printing-vault");
  const done = () => { sheet.remove(); document.body.classList.remove("printing-vault"); window.removeEventListener("afterprint", done); };
  window.addEventListener("afterprint", done);
  window.print();
  setTimeout(done, 60_000);
}

async function showVaultWords() {
  let words;
  try {
    words = await invoke("vault_words");
  } catch (e) {
    micToast(String(e));
    return;
  }
  vaultSheet(tr("Your recovery words"), (dialog, body) => {
    const show = () => {
      const grid = el("ol", { class: "vault-words" }, ...words.map((w) => el("li", {}, w)));
      const copy = el("button", { class: "pill" }, tr("Copy"));
      copy.addEventListener("click", async () => {
        await navigator.clipboard.writeText(words.map((w, i) => `${i + 1}. ${w}`).join("\n")).catch(() => {});
        copy.textContent = tr("Copied");
      });
      const save = el("button", { class: "pill" }, tr("Save kit…"));
      save.addEventListener("click", () => vaultTry(save, body, async () => {
        const path = await window.__TAURI__.dialog.save({ title: tr("Save recovery kit"), defaultPath: `Voice-OS recovery kit ${vault?.id ?? ""}.txt`.replace(" .txt", ".txt"), filters: [{ name: tr("Text"), extensions: ["txt"] }] });
        if (!path) return;
        await invoke("vault_save_kit", { path });
        save.textContent = tr("Saved");
      }));
      const print = el("button", { class: "pill" }, tr("Print kit"));
      print.addEventListener("click", () => vaultTry(print, body, async () => vaultPrint(await invoke("vault_kit"))));
      const next = el("button", { class: "primary pill", onclick: () => quiz() }, tr("I've saved them"));
      body.replaceChildren(
        el("p", { class: "hint" }, tr("These 24 words open your vault{0}, on this computer or any other. Voice-OS opens it itself day to day; the words are for when it can't. ", { "0": vault?.id ? tr(" (ID {id})", { id: vault.id }) : "" }),
          el("strong", {}, tr("Anyone with them can read your notes, and without them a vault this computer can't open stays shut."))),
        grid,
        el("p", { class: "hint" }, tr("Print the kit and file it, or keep the words in a password manager. Don't keep them only on this computer. The kit says what to do on a new computer.")),
        el("div", { class: "actions" }, copy, isDesktop() ? save : null, isDesktop() ? print : null, el("span", { class: "grow" }),
          el("button", { class: "pill", onclick: () => dialog.close() }, tr("Close")), next));
    };
    // Three of the words are asked back, so a copy that was never made is found now and not when it's needed.
    const quiz = () => {
      const asked = [...words.keys()].sort(() => Math.random() - 0.5).slice(0, 3).sort((a, b) => a - b);
      const boxes = asked.map((i) => el("input", { type: "text", autocomplete: "off", autocapitalize: "off", spellcheck: "false", "aria-label": tr("Word {0}", { "0": i + 1 }) }));
      const confirm = el("button", { class: "primary pill" }, tr("Confirm"));
      confirm.addEventListener("click", () => vaultTry(confirm, body, async () => {
        const wrong = asked.filter((i, k) => boxes[k].value.trim().toLowerCase().slice(0, 4) !== words[i].slice(0, 4));
        if (wrong.length) throw tr("Word {0} doesn't match. Check your copy.", { "0": trList(wrong.map((i) => i + 1)) });
        vault = await invoke("vault_words_saved");
        dialog.close();
        await vaultChanged();
        micToast(tr("Good. Your recovery words are saved."));
      }));
      boxes.at(-1).addEventListener("keydown", (e) => { if (e.key === "Enter") confirm.click(); });
      body.replaceChildren(
        el("p", { class: "hint" }, tr("From your copy, type these three words.")),
        ...asked.map((i, k) => el("label", { class: "field" }, el("span", {}, tr("Word {0}", { "0": i + 1 })), boxes[k])),
        el("div", { class: "actions" }, el("button", { class: "pill", onclick: () => show() }, tr("Show the words again")), el("span", { class: "grow" }), confirm));
      boxes[0].focus();
    };
    show();
  }, { wide: true });
}

/// Checks the person's copy of the words against the vault, without showing them.
function showVaultCheck() {
  vaultSheet(tr("Check your recovery words"), (dialog, body) => {
    const [box, count] = vaultWordsBox(tr("Type the 24 words from your copy"));
    const check = el("button", { class: "primary pill" }, tr("Check"));
    check.addEventListener("click", async () => {
      const status = await vaultTry(check, body, () => invoke("vault_check_words", { words: box.value }));
      if (!status) return;
      vault = status;
      dialog.close();
      await vaultChanged();
      micToast(tr("Those are the right words for this vault."));
    });
    vaultAdd(body, el("p", { class: "hint" }, tr("Type the words from where you keep them. Voice-OS says whether they open this vault, so you know your copy is good.")),
      el("label", { class: "field" }, el("span", {}, tr("Recovery words")), box, count),
      el("div", { class: "actions" }, el("button", { class: "pill", onclick: () => dialog.close() }, tr("Cancel")), check));
    box.focus();
  });
}

// ---------- The vault password ----------

function showVaultPassword() {
  vaultSheet(vault?.password ? tr("Change vault password") : tr("Set vault password"), (dialog, body) => {
    const field = (label) => el("input", { type: "password", autocomplete: "new-password", "aria-label": label });
    const [first, again] = [field(tr("Password")), field(tr("Password again"))];
    const hint = el("input", { type: "text", autocomplete: "off", maxlength: "120", "aria-label": tr("Hint"), value: vault?.hint ?? "" });
    const save = el("button", { class: "primary pill" }, tr("Save"));
    save.addEventListener("click", async () => {
      const status = await vaultTry(save, body, async () => {
        if (first.value !== again.value) throw tr("Passwords don't match.");
        if (!first.value) throw tr("Type a password.");
        return invoke("vault_set_password", { password: first.value, hint: hint.value });
      });
      if (!status) return;
      vault = status;
      dialog.close();
      await vaultChanged();
      micToast(tr("Vault password set."));
    });
    vaultAdd(body, 
      el("p", { class: "hint" }, tr("A second way back in, for when this computer can't open the vault and the recovery words aren't to hand. It also opens daily copies in your backup folder. Not asked for day to day. Choose one you'll remember for years: a long phrase beats a clever one.")),
      el("label", { class: "field" }, el("span", {}, tr("Password"), el("small", {}, tr("At least 8 characters."))), first),
      el("label", { class: "field" }, el("span", {}, tr("Password again")), again),
      el("label", { class: "field" }, el("span", {}, tr("Hint"), el("small", {}, tr("Shown where the password is asked, to anyone with the vault. Something only you would understand."))), hint),
      el("div", { class: "actions" }, el("button", { class: "pill", onclick: () => dialog.close() }, tr("Cancel")), save));
    first.focus();
  });
}

// ---------- Bringing in from a backup, another computer or another vault ----------

/// What came in, said in one line.
function broughtText(r) {
  const parts = [r.notes ? vaultPlural(r.notes, "note") : null, r.books ? vaultPlural(r.books, "book") : null, r.meetings ? vaultPlural(r.meetings, "meeting") : null].filter(Boolean);
  let text = r.adopted ? tr("This computer now uses vault {other_vault}", { other_vault: r.other_vault }) + (parts.length ? `, with ${parts.join(", ")}.` : ".")
    : parts.length ? tr("Brought in {0}.", { "0": parts.join(", ") }) : tr("Nothing new: everything found is already here.");
  if (r.had && !r.adopted) text += ` ${vaultPlural(r.had, "item")} already here ${r.had === 1 ? "was" : "were"} left as ${r.had === 1 ? "it was" : "they were"}.`;
  if (r.other_vault && !r.adopted) text += tr(" What vault {other_vault} sealed now opens here.", { other_vault: r.other_vault });
  return text;
}

/// Chooses where to bring things in from: a folder (a backup, a copy of Documents › Voice-OS, or
/// Voice's data folder) or a vault file.
async function chooseBringIn(folder) {
  try {
    const path = await window.__TAURI__.dialog.open(folder
      ? { directory: true, title: tr("Choose a backup or Voice-OS folder"), defaultPath: vault?.backup_dir }
      : { title: tr("Choose a vault file"), filters: [{ name: tr("Vault"), extensions: ["db"] }] });
    if (path) showVaultBringIn({ path, folder });
  } catch (e) {
    micToast(String(e));
  }
}

/// Opens what was chosen with its words or password, then either takes it on as this computer's vault (a
/// new computer) or adds what's in it to this one.
function showVaultBringIn({ path, folder, adopt = false }) {
  vaultSheet(adopt ? tr("Open your vault on this computer") : tr("Bring in from backup"), (dialog, body) => {
    const [box, count] = vaultWordsBox(tr("That vault's 24 recovery words"));
    const password = el("input", { type: "password", autocomplete: "off", "aria-label": tr("That vault's password") });
    const run = (command, button) => async () => {
      const result = await vaultTry(button, body, () => invoke(command, { path, words: box.value, password: password.value }));
      if (!result) return;
      dialog.close();
      await vaultChanged();
      vaultSheet(tr("Done"), (done, inner) => {
        vaultAdd(inner, el("p", { class: "hint" }, broughtText(result)),
          result.failed?.length ? el("div", { class: "notice" }, tr("Couldn't bring in: {0}.", { "0": result.failed.join("; ") })) : null,
          result.adopted ? el("p", { class: "hint" }, tr("Your recovery words for that vault keep working, and this computer opens it itself from now on.")) : null,
          !result.adopted && result.notes + result.books > 0 ? el("p", { class: "hint" }, tr("This vault's recovery words are unchanged.")) : null,
          el("div", { class: "actions" }, el("button", { class: "primary pill", onclick: () => done.close() }, tr("OK"))));
      });
    };
    const take = el("button", { class: adopt ? "primary pill" : "pill" }, tr("Use it on this computer"));
    const add = el("button", { class: adopt ? "pill" : "primary pill" }, tr("Add its contents here"));
    take.addEventListener("click", run("vault_adopt", take));
    add.addEventListener("click", run(folder ? "vault_bring_in_folder" : "vault_bring_in", add));
    const fresh = (vault?.own_notes ?? 0) === 0;
    vaultAdd(body, 
      el("p", { class: "vault-path" }, path),
      el("p", { class: "hint" }, folder
        ? tr("Voice-OS looks in this folder for the newest vault copy, notes and books from before the vault, and meetings. Nothing in the folder is changed.")
        : tr("A vault file: a daily copy from Vault backups, a vault set aside, or one from another computer.")),
      el("label", { class: "field" }, el("span", {}, tr("That vault's recovery words"), el("small", {}, tr("Leave empty for a copy of this vault, or notes from before the vault."))), box, count),
      el("details", { class: "vault-more" }, el("summary", {}, tr("Use its vault password instead")),
        el("label", { class: "field" }, el("span", {}, tr("That vault's password"), el("small", {}, tr("Works for a copy from a Vault backups folder, which keeps the key file the password needs."))), password)),
      vault?.sealed ? el("div", { class: "vault-choice" },
        el("div", {}, el("strong", {}, tr("Use it on this computer")), el("small", {}, fresh
          ? tr("For a new computer. That vault becomes this computer's, and its recovery words keep working.")
          : tr("That vault becomes this computer's and its words keep working. Everything here is added to it; this vault is set aside, not deleted."))),
        el("div", {}, el("strong", {}, tr("Add its contents here")), el("small", {}, tr("Its notes, books and meetings join this vault, which keeps its own recovery words. Anything already here is left as is.")))) : null,
      el("div", { class: "actions" }, el("button", { class: "pill", onclick: () => dialog.close() }, tr("Cancel")), el("span", { class: "grow" }), vault?.sealed ? take : null, add));
    box.focus();
  }, { wide: true });
}

/// Meetings copied from another computer, sealed by its vault, open here once its words are added.
function showVaultAddWords() {
  vaultSheet(tr("Open meetings from another vault"), (dialog, body) => {
    const [box, count] = vaultWordsBox(tr("The other vault's 24 recovery words"));
    const add = el("button", { class: "primary pill" }, tr("Add"));
    add.addEventListener("click", async () => {
      const id = await vaultTry(add, body, () => invoke("vault_add_words", { words: box.value }));
      if (!id) return;
      dialog.close();
      await vaultChanged();
      micToast(tr("Meetings and files sealed by vault {id} open here now.", { id: id }));
    });
    vaultAdd(body, el("p", { class: "hint" }, tr("Meetings shown as “Locked meeting (from another vault)” were sealed on another computer or by a replaced vault. Type that vault's words and they open here. The words are kept inside this vault, encrypted.")),
      el("label", { class: "field" }, el("span", {}, tr("Recovery words")), box, count),
      el("div", { class: "actions" }, el("button", { class: "pill", onclick: () => dialog.close() }, tr("Cancel")), add));
    box.focus();
  });
}

async function chooseBackupFolder(page) {
  try {
    const path = await window.__TAURI__.dialog.open({ directory: true, title: tr("Keep a daily vault copy in this folder"), defaultPath: vault?.backup_dir });
    if (!path) return;
    vault = await invoke("vault_set_backup_dir", { folder: path });
    showVaultSettings();
    micToast(tr("A copy is there now, and one goes there daily."));
  } catch (e) {
    showError(page, e);
  }
}

// ---------- Settings › Vault ----------

function vaultSummary() {
  if (!vault) return "";
  if (vault.state === "locked") return tr("Locked");
  if (vault.state === "damaged") return tr("Needs restoring");
  if (!vault.sealed) return tr("Not encrypted on this device yet");
  return vault.words_saved ? tr("Encrypted · recovery words saved") : tr("Encrypted · recovery words not saved");
}

async function showVaultSettings() {
  leave();
  showPage();
  view = { kind: "vault" };
  refreshItems();
  await refreshVault();
  const page = el("div", { class: "page" }, el("button", { class: "link back-link", onclick: () => showSettings() }, tr("‹ Settings")), el("h1", {}, tr("Vault")));
  const row = (label, detail, ...actions) => el("div", { class: "row" }, el("span", { class: "label" }, label, detail ? el("small", {}, detail) : null), ...actions.filter(Boolean));
  const button = (text, onclick, kind = "pill") => el("button", { class: kind, onclick }, text);
  if (!vault) {
    vaultAdd(page, el("div", { class: "error", role: "alert" }, tr("Couldn't read the vault's state.")));
    main.replaceChildren(page);
    return;
  }
  if (vault.state !== "open") {
    const damaged = vault.state === "damaged";
    vaultAdd(page, el("div", { class: "notice" }, el("strong", {}, damaged ? tr("The vault can't be read. ") : tr("Your notes, books and meetings are locked. ")),
      damaged ? tr("An earlier copy can replace it.") : tr("This computer can't open the vault itself; your recovery words or password can.")),
      el("div", { class: "actions" }, damaged ? button(tr("Restore a copy"), () => showVaultCopies(), "primary pill") : button(tr("Open"), () => showVaultUnlock(), "primary pill")));
    main.replaceChildren(page);
    return;
  }
  const counts = [vaultPlural(vault.own_notes, "note"), vaultPlural(vault.books, "book")].join(" · ");
  if (!vault.sealed) {
    vaultAdd(page, el("p", { class: "hint" }, tr("Your notes, books and what's written about them ({counts}) are in one database on this {1}. Encryption comes to Windows first; it isn't encrypted here yet.", { counts: counts, "1": deviceWord() })));
  } else {
    vaultAdd(page, 
      el("p", { class: "hint" }, tr("Your notes, books, bookmarks, chats, analyses and translations ({counts}) are in one encrypted database. Meetings, and pictures and files in notes and books, are sealed with the same key. This computer opens it itself, for your Windows account only. Copied anywhere else, none of it can be read without one of the ways back in below.", { counts: counts })),
      el("p", { class: "hint vault-id-line" }, tr("Vault ID "), el("strong", {}, vault.id), tr(" · your recovery kit and backups carry the same ID.")),
      vault.sealing ? el("div", { class: "notice" }, tr("Sealing earlier recordings: {0} to go. It continues in the background and resumes next time Voice-OS starts.", { "0": vaultPlural(vault.sealing, "file") })) : null);

    // Ways back in: each with whether it's ready.
    const sixMonths = 183 * 24 * 3600;
    const stale = vault.words_saved && Date.now() / 1000 - vault.words_checked > sixMonths;
    const backupAge = vault.backup_latest ? Date.now() / 1000 - vault.backup_latest : Infinity;
    const ready = [true, !!vault.words_saved, vault.password, backupAge < 3 * 24 * 3600].filter(Boolean).length;
    const mark = (ok) => el("span", { class: "vault-mark" + (ok ? " ok" : ""), "aria-label": ok ? tr("Ready") : tr("Not set up") }, ok ? "✓" : "!");
    const way = (ok, label, detail, ...actions) => el("div", { class: "row vault-way" }, mark(ok), el("span", { class: "label" }, label, el("small", {}, detail)), ...actions.filter(Boolean));
    vaultAdd(page, el("h2", {}, tr("Ways back in")),
      el("p", { class: "hint" }, tr("{ready} of 4 ready. Any one opens your vault; the more you have, the less one lost matters.", { ready: ready })),
      el("div", { class: "group" },
        way(true, tr("This Windows account"), tr("Opens the vault itself day to day. Lost if Windows is reinstalled or this computer replaced.")),
        way(!!vault.words_saved, tr("Recovery words"), vault.words_saved
          ? tr("Saved {0} · last checked {1}{2}", { "0": vaultDate(vault.words_saved), "1": vaultDate(vault.words_checked), "2": stale ? tr(" · worth checking your copy again") : "" })
          : tr("24 words that open the vault on any computer. Not saved yet."),
          vault.words_saved ? button(tr("Check my copy"), () => showVaultCheck()) : null,
          button(vault.words_saved ? tr("Show") : tr("Show and save"), () => showVaultWords(), vault.words_saved ? "pill" : "primary pill")),
        way(vault.password, tr("Vault password"), vault.password ? tr("Set{0}", { "0": vault.hint ? tr(" · hint: {hint}", { hint: vault.hint }) : "" }) : tr("Optional: a password you choose, with a hint for yourself."),
          vault.password ? button(tr("Remove"), async (e) => {
            const status = await vaultTry(e.currentTarget, page, () => invoke("vault_set_password", { password: "", hint: "" }));
            if (status) { vault = status; showVaultSettings(); }
          }) : null,
          button(vault.password ? tr("Change") : tr("Set a password"), () => showVaultPassword())),
        way(backupAge < 3 * 24 * 3600, tr("A copy that outlives this computer"),
          (vault.backup_latest ? tr("Latest {0}", { "0": vaultWhen(vault.backup_latest) }) : tr("None yet")) + ` · ${vault.backup_dir}`
            + (vault.backup_reachable ? "" : tr(" · that folder can't be reached now")) + (vault.backup_chosen ? "" : tr(" · keep Documents backed up or synced, or choose a folder that is")),
          button(tr("Choose a folder…"), () => chooseBackupFolder(page)), button(tr("Open"), () => invoke("vault_open_backups").catch((e) => showError(page, e))))));
    if (!vault.words_saved) {
      vaultAdd(page, el("div", { class: "notice" }, el("strong", {}, tr("Save your recovery words. ")),
        tr("If Windows is reinstalled or this computer replaced, they're the way back into your notes, books and meetings.")));
    }
  }
  const latest = vault.snapshots[0];
  const copyNow = el("button", { class: "pill" }, tr("Copy now"));
  copyNow.addEventListener("click", async () => {
    const status = await vaultTry(copyNow, page, () => invoke("vault_copy_now"));
    if (status) { vault = status; showVaultSettings(); micToast(tr("Copied here and to your backup folder.")); }
  });
  vaultAdd(page, el("h2", {}, tr("Copies on this computer")), el("div", { class: "group" },
    row(latest ? tr("Latest {0}", { "0": vaultWhen(latest.at) }) : tr("No copy yet"), latest ? tr("{0} kept: the latest dozen, one a day for a fortnight, one a month for a year.", { "0": vaultPlural(vault.snapshots.length, "copy", "copies") }) : tr("One is made within the hour once there are notes."),
      copyNow, vault.snapshots.length ? button(tr("Restore a copy…"), () => showVaultCopies()) : null)));
  if (isDesktop()) {
    vaultAdd(page, el("h2", {}, tr("Bring in")), el("div", { class: "group" },
      row(tr("From a backup or another computer"), tr("A Vault backups folder, a copy of Documents › Voice-OS, or Voice-OS's data folder from before the vault. Brings notes, books and meetings."),
        button(tr("Choose a folder…"), () => chooseBringIn(true))),
      row(tr("From a vault file"), tr("A single daily copy, or a vault set aside."), button(tr("Choose a file…"), () => chooseBringIn(false))),
      vault.sealed ? row(tr("Meetings from another vault"), vault.keyring ? tr("This vault also opens what {0} sealed.", { "0": vaultPlural(vault.keyring, "other vault") }) : tr("Open meetings sealed on another computer, with that vault's words."),
        button(tr("Add its words…"), () => showVaultAddWords())) : null));
  }
  vaultAdd(page, el("h2", {}, tr("Where it is")), el("div", { class: "group" }, row(tr("The vault"), vault.folder), row(tr("Meetings"), tr("Documents › Voice-OS, sealed beside the vault's daily copies"))));
  main.replaceChildren(page);
}
