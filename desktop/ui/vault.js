// The vault: getting back into it, the recovery words, the vault password, the sealed copies, and bringing
// notes, books and meetings in from a backup or another computer.

let vault = null; // what `vault_status` last said

async function refreshVault() {
  vault = await invoke("vault_status").catch(() => null);
  return vault;
}

const vaultOpen = () => !vault || vault.state === "open";
const vaultPlural = (n, word, many = `${word}s`) => `${n} ${n === 1 ? word : many}`;

/// Runs `open` when the vault is open; otherwise shows the way back in.
function vaultGate(open) {
  return () => (vaultOpen() ? open() : vault.state === "damaged" ? showVaultCopies() : showVaultUnlock());
}

function vaultDate(unix) {
  return new Date(unix * 1000).toLocaleDateString("en-GB", { day: "numeric", month: "short", year: "numeric" });
}

function vaultWhen(unix) {
  return new Date(unix * 1000).toLocaleString("en-GB", { day: "numeric", month: "short", year: "numeric", hour: "2-digit", minute: "2-digit" });
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
  const card = (title, text, ...actions) => el("div", { class: "optimise-nudge vault-card", role: "region", "aria-label": "Vault" },
    svg("lock", 22), el("div", { class: "grow" }, el("strong", {}, title), el("small", {}, text)), el("div", { class: "optimise-nudge-actions" }, ...actions));
  if (vault.state === "locked") {
    return card("Your notes, books and meetings are locked", "This computer can't open your vault by itself. Your recovery words or vault password open it.",
      el("button", { class: "primary pill", onclick: () => showVaultUnlock() }, "Open them"));
  }
  if (vault.state === "damaged") {
    return card("Your vault needs bringing back", "The vault can't be read, so notes, books and meetings are shut. An earlier copy can take its place.",
      el("button", { class: "primary pill", onclick: () => showVaultCopies() }, "Bring back a copy"));
  }
  // A new computer whose synced Documents holds the vault from the old one.
  const other = vault.others?.[0];
  if (vault.sealed && other && vault.own_notes === 0) {
    return card("Your vault from another computer is here", `A copy from ${vaultDate(other.at)} (vault ${vaultId(other.id)}) is in your backups. Its recovery words open it on this computer.`,
      el("button", { class: "primary pill", onclick: () => showVaultBringIn({ path: vault.backup_dir, folder: true, adopt: true }) }, "Open it here"));
  }
  if (vault.moved && vault.sealed) {
    const later = el("button", { class: "link" }, "Got it");
    const made = card("Your notes are now in an encrypted vault", movedText(vault.moved),
      el("button", { class: "primary pill", onclick: () => showVaultWelcome() }, "What this means"), later);
    later.addEventListener("click", async () => { await invoke("vault_moved_seen").catch(() => {}); made.remove(); refreshVault(); });
    return made;
  }
  if (vault.remind) {
    const later = el("button", { class: "link" }, "Not now");
    const made = card("Save your recovery words", "Your notes, books and meetings are encrypted, and only this computer opens them by itself. 24 words get you back in if it ever can't.",
      el("button", { class: "primary pill", onclick: () => showVaultWords() }, "Show the words"), later);
    later.addEventListener("click", async () => { await invoke("vault_remind_later").catch(() => {}); made.remove(); refreshVault(); });
    return made;
  }
  return null;
}

function movedText(moved) {
  const parts = [moved.notes ? vaultPlural(moved.notes, "note") : null, moved.books ? vaultPlural(moved.books, "book") : null].filter(Boolean);
  const what = parts.length ? `${parts.join(" and ")} moved in` : "Your notes were moved in";
  const left = vault?.sealing ? ` Meetings are being sealed in the background: ${vaultPlural(vault.sealing, "file")} to go.` : " Meetings are sealed too.";
  return `${what}.${left}`;
}

/// Once, after this version moves the person's notes, books and meetings into the vault: what changed,
/// and the one thing worth doing now.
function showVaultWelcome() {
  vaultSheet("Your notes are now in an encrypted vault", (dialog, body) => {
    const moved = vault?.moved ?? { notes: 0, books: 0, failed: [] };
    const seen = () => invoke("vault_moved_seen").catch(() => {});
    dialog.addEventListener("close", seen);
    vaultAdd(body, 
      el("p", { class: "hint" }, `${movedText(moved)} Everything was read back and checked before the old files were removed.`),
      el("ul", { class: "vault-list" },
        el("li", {}, el("strong", {}, "Nothing changes day to day. "), "Voice opens the vault by itself for your Windows account; there's no password to type."),
        el("li", {}, el("strong", {}, "Copies can't be read. "), "A backup, a synced folder or a copied disk holds only sealed files."),
        el("li", {}, el("strong", {}, "A sealed copy is kept every day "), `in ${vault?.backup_dir ?? "Documents › Superteem Voice › Vault backups"}.`),
        el("li", {}, el("strong", {}, "Meetings open only in Voice. "), "Export writes readable copies when you need them elsewhere.")),
      moved.failed?.length ? el("div", { class: "notice" }, `Not moved, and left where they were: ${moved.failed.join(", ")}.`) : null,
      el("div", { class: "notice" }, el("strong", {}, "One thing to do now: save your recovery words. "),
        "If Windows is reinstalled or this computer is replaced, they're how you get back in. Nobody else, Superteem included, can open your vault."),
      el("div", { class: "actions" }, el("button", { class: "pill", onclick: () => dialog.close() }, "Later"),
        el("button", { class: "primary pill", onclick: () => { dialog.close(); showVaultWords(); } }, "Save my recovery words")));
  });
}

/// The line in the sidebar when nothing can be listed.
function vaultItem() {
  if (vaultOpen()) return null;
  const damaged = vault.state === "damaged";
  return el("button", { class: "item vault-item", onclick: () => (damaged ? showVaultCopies() : showVaultUnlock()) },
    el("span", { class: "icon" }, svg("lock", 16)),
    el("span", { class: "body" }, el("span", { class: "title" }, damaged ? "Vault needs bringing back" : "Notes, books and meetings are locked"),
      el("span", { class: "sub" }, damaged ? "Bring back a copy" : "Open with your recovery words")));
}

// ---------- Getting back in ----------

/// A box for 24 recovery words that says how many are in it, and which vault they open, as they're typed.
function vaultWordsBox(placeholder) {
  const box = el("textarea", { rows: "5", spellcheck: "false", autocapitalize: "off", autocomplete: "off", "aria-label": "Recovery words", placeholder });
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
      count.textContent = `24 words · for vault ${id}`;
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
  vaultSheet("Open your notes, books and meetings", (dialog, body) => {
    const [box, count] = vaultWordsBox("Type or paste your 24 recovery words");
    const open = el("button", { class: "primary pill" }, "Open");
    const done = async (status) => {
      if (!status) return;
      vault = status;
      dialog.close();
      await vaultChanged();
      if (vault.state === "damaged") showVaultCopies();
      else micToast("Your vault is open, and this computer will open it by itself from now on.");
    };
    open.addEventListener("click", async () => done(await vaultTry(open, body, () => invoke("vault_unlock_words", { words: box.value }))));
    vaultAdd(body, 
      el("p", { class: "hint" }, "This computer can't open your vault by itself. That happens after Windows is reinstalled, after your Windows password is reset by someone else, or when the vault came from another computer. Nothing is lost: your notes, books and meetings are still here, encrypted."),
      el("label", { class: "field" }, el("span", {}, "Recovery words", el("small", {}, "The 24 words from your recovery kit. Capitals and spacing don't matter; the first four letters of each word are enough.")), box, count),
      el("div", { class: "actions" }, open));
    if (vault?.password) {
      const password = el("input", { type: "password", autocomplete: "off", "aria-label": "Vault password" });
      const use = el("button", { class: "pill" }, "Open with the password");
      use.addEventListener("click", async () => done(await vaultTry(use, body, () => invoke("vault_unlock_password", { password: password.value }))));
      password.addEventListener("keydown", (e) => { if (e.key === "Enter") use.click(); });
      vaultAdd(body, el("div", { class: "vault-or" }, "or"),
        el("label", { class: "field" }, el("span", {}, "Vault password", vault.hint ? el("small", {}, `Your hint: ${vault.hint}`) : null), password),
        el("div", { class: "actions" }, use));
    }
    const fresh = el("button", { class: "pill danger" }, "Start an empty vault");
    fresh.addEventListener("click", async () => {
      if (fresh.dataset.sure !== "yes") {
        fresh.dataset.sure = "yes";
        fresh.textContent = "Yes, start an empty vault";
        return;
      }
      const status = await vaultTry(fresh, body, () => invoke("vault_start_new"));
      if (!status) return;
      vault = status;
      dialog.close();
      await vaultChanged();
      micToast("A new vault is open. The old one was set aside, not deleted.");
    });
    vaultAdd(body, el("details", { class: "vault-more" }, el("summary", {}, "I don't have the words"),
      el("p", { class: "hint" }, "Look for a file or printout called “Voice recovery kit”, a password manager entry, or a photo of 24 numbered words. The kit names the vault ID its words are for. ",
        "If this vault was on another computer that still works, open Voice there: Settings › Vault shows the words again."),
      el("p", { class: "hint" }, "Without the words or the password, nobody can open this vault, Superteem included. You can start an empty one and keep using Voice. ",
        "The locked vault is set aside in its folder, untouched, and its meetings stay where they are, shown as locked. Both open again if the words turn up: Settings › Vault › Bring in from a backup."),
      el("div", { class: "actions" }, fresh)),
      el("div", { class: "actions" }, el("button", { class: "pill", onclick: () => dialog.close() }, "Close")));
    box.focus();
  });
}

/// The sealed copies on this computer, to bring one back in place of the vault.
function showVaultCopies() {
  vaultSheet(vault?.state === "damaged" ? "Bring your vault back" : "Bring back a copy", (dialog, body) => {
    const damaged = vault?.state === "damaged";
    vaultAdd(body, el("p", { class: "hint" }, damaged
      ? `The vault can't be read${vault.detail ? ` (${vault.detail.replace(/\.$/, "")})` : ""}. Voice keeps sealed copies of it as it changes; choose one to put in its place. The unreadable vault is set aside, not deleted.`
      : "Voice keeps sealed copies of the vault as it changes. Bringing one back puts every note and book as it was then. How the vault is now is kept as a copy first, so this can be undone."));
    if (!vault?.snapshots.length) {
      vaultAdd(body, el("p", { class: "notice" }, "There are no copies on this computer yet."),
        damaged ? el("p", { class: "hint" }, "If Documents is backed up or synced, a daily copy is in Documents › Superteem Voice › Vault backups. Open the vault, or start an empty one, then use Settings › Vault › Bring in from a backup.") : null);
    }
    const list = el("div", { class: "group vault-copies" });
    for (const copy of vault?.snapshots ?? []) {
      const back = el("button", { class: "pill" }, "Bring back");
      back.addEventListener("click", async () => {
        if (back.dataset.sure !== "yes" && !damaged) {
          back.dataset.sure = "yes";
          back.textContent = "Yes, bring it back";
          return;
        }
        const status = await vaultTry(back, body, () => invoke("vault_restore", { file: copy.file }));
        if (!status) return;
        vault = status;
        dialog.close();
        await vaultChanged();
        micToast(`Your notes are as they were on ${vaultWhen(copy.at)}.`);
      });
      list.append(el("div", { class: "row" }, el("span", { class: "label" }, vaultWhen(copy.at), el("small", {}, vaultSize(copy.bytes))), back));
    }
    if (list.children.length) vaultAdd(body, list);
    if (damaged && !vault.key_known) vaultAdd(body, el("p", { class: "hint" }, "Open the vault with your recovery words first, then bring a copy back."));
    vaultAdd(body, el("div", { class: "actions" },
      damaged ? el("button", { class: "pill", onclick: () => { dialog.close(); showVaultUnlock(); } }, "Other ways in") : null,
      el("button", { class: "pill", onclick: () => dialog.close() }, "Close")));
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
  vaultSheet("Your recovery words", (dialog, body) => {
    const show = () => {
      const grid = el("ol", { class: "vault-words" }, ...words.map((w) => el("li", {}, w)));
      const copy = el("button", { class: "pill" }, "Copy");
      copy.addEventListener("click", async () => {
        await navigator.clipboard.writeText(words.map((w, i) => `${i + 1}. ${w}`).join("\n")).catch(() => {});
        copy.textContent = "Copied";
      });
      const save = el("button", { class: "pill" }, "Save the kit…");
      save.addEventListener("click", () => vaultTry(save, body, async () => {
        const path = await window.__TAURI__.dialog.save({ title: "Save the recovery kit", defaultPath: `Voice recovery kit ${vault?.id ?? ""}.txt`.replace(" .txt", ".txt"), filters: [{ name: "Text", extensions: ["txt"] }] });
        if (!path) return;
        await invoke("vault_save_kit", { path });
        save.textContent = "Saved";
      }));
      const print = el("button", { class: "pill" }, "Print the kit");
      print.addEventListener("click", () => vaultTry(print, body, async () => vaultPrint(await invoke("vault_kit"))));
      const next = el("button", { class: "primary pill", onclick: () => quiz() }, "I've saved them");
      body.replaceChildren(
        el("p", { class: "hint" }, `These 24 words open your vault${vault?.id ? ` (ID ${vault.id})` : ""}, on this computer or any other. Voice opens it by itself day to day; the words are for when it can't. `,
          el("strong", {}, "Anyone who has them can read your notes, and without them a vault this computer can't open stays shut.")),
        grid,
        el("p", { class: "hint" }, "Print the kit and file it, or keep the words in a password manager. Don't keep them only on this computer. The kit also says what to do on a new computer."),
        el("div", { class: "actions" }, copy, isDesktop() ? save : null, isDesktop() ? print : null, el("span", { class: "grow" }),
          el("button", { class: "pill", onclick: () => dialog.close() }, "Close"), next));
    };
    // Three of the words are asked back, so a copy that was never made is found now and not when it's needed.
    const quiz = () => {
      const asked = [...words.keys()].sort(() => Math.random() - 0.5).slice(0, 3).sort((a, b) => a - b);
      const boxes = asked.map((i) => el("input", { type: "text", autocomplete: "off", autocapitalize: "off", spellcheck: "false", "aria-label": `Word ${i + 1}` }));
      const confirm = el("button", { class: "primary pill" }, "Confirm");
      confirm.addEventListener("click", () => vaultTry(confirm, body, async () => {
        const wrong = asked.filter((i, k) => boxes[k].value.trim().toLowerCase().slice(0, 4) !== words[i].slice(0, 4));
        if (wrong.length) throw `Word ${wrong.map((i) => i + 1).join(" and ")} doesn't match. Check your copy against the words.`;
        vault = await invoke("vault_words_saved");
        dialog.close();
        await vaultChanged();
        micToast("Good. Your recovery words are saved.");
      }));
      boxes.at(-1).addEventListener("keydown", (e) => { if (e.key === "Enter") confirm.click(); });
      body.replaceChildren(
        el("p", { class: "hint" }, "From the copy you just made, type these three words."),
        ...asked.map((i, k) => el("label", { class: "field" }, el("span", {}, `Word ${i + 1}`), boxes[k])),
        el("div", { class: "actions" }, el("button", { class: "pill", onclick: () => show() }, "Show the words again"), el("span", { class: "grow" }), confirm));
      boxes[0].focus();
    };
    show();
  }, { wide: true });
}

/// Checks the person's copy of the words against the vault, without showing them.
function showVaultCheck() {
  vaultSheet("Check your recovery words", (dialog, body) => {
    const [box, count] = vaultWordsBox("Type the 24 words from your copy");
    const check = el("button", { class: "primary pill" }, "Check");
    check.addEventListener("click", async () => {
      const status = await vaultTry(check, body, () => invoke("vault_check_words", { words: box.value }));
      if (!status) return;
      vault = status;
      dialog.close();
      await vaultChanged();
      micToast("Those are the right words. They open this vault.");
    });
    vaultAdd(body, el("p", { class: "hint" }, "Type the words from where you keep them. Voice says whether they open this vault, so you know your copy is good before you need it."),
      el("label", { class: "field" }, el("span", {}, "Recovery words"), box, count),
      el("div", { class: "actions" }, el("button", { class: "pill", onclick: () => dialog.close() }, "Cancel"), check));
    box.focus();
  });
}

// ---------- The vault password ----------

function showVaultPassword() {
  vaultSheet(vault?.password ? "Change the vault password" : "Set a vault password", (dialog, body) => {
    const field = (label) => el("input", { type: "password", autocomplete: "new-password", "aria-label": label });
    const [first, again] = [field("Password"), field("Password again")];
    const hint = el("input", { type: "text", autocomplete: "off", maxlength: "120", "aria-label": "Hint", value: vault?.hint ?? "" });
    const save = el("button", { class: "primary pill" }, "Save");
    save.addEventListener("click", async () => {
      const status = await vaultTry(save, body, async () => {
        if (first.value !== again.value) throw "The two passwords aren't the same.";
        if (!first.value) throw "Type a password.";
        return invoke("vault_set_password", { password: first.value, hint: hint.value });
      });
      if (!status) return;
      vault = status;
      dialog.close();
      await vaultChanged();
      micToast("The vault password is set.");
    });
    vaultAdd(body, 
      el("p", { class: "hint" }, "A second way back in, for when this computer can't open the vault and the recovery words aren't to hand. It also opens the daily copies in your backup folder. It isn't asked for day to day. Choose one you'll still know in years: a long phrase is better than a clever one."),
      el("label", { class: "field" }, el("span", {}, "Password", el("small", {}, "At least 8 characters.")), first),
      el("label", { class: "field" }, el("span", {}, "Password again"), again),
      el("label", { class: "field" }, el("span", {}, "Hint", el("small", {}, "Shown where the password is asked for, to anyone who has the vault. Something only you would understand.")), hint),
      el("div", { class: "actions" }, el("button", { class: "pill", onclick: () => dialog.close() }, "Cancel"), save));
    first.focus();
  });
}

// ---------- Bringing in from a backup, another computer or another vault ----------

/// What came in, said in one line.
function broughtText(r) {
  const parts = [r.notes ? vaultPlural(r.notes, "note") : null, r.books ? vaultPlural(r.books, "book") : null, r.meetings ? vaultPlural(r.meetings, "meeting") : null].filter(Boolean);
  let text = r.adopted ? `This computer now uses vault ${r.other_vault}` + (parts.length ? `, with ${parts.join(", ")}.` : ".")
    : parts.length ? `Brought in ${parts.join(", ")}.` : "Nothing new: everything found is already here.";
  if (r.had && !r.adopted) text += ` ${vaultPlural(r.had, "item")} already here ${r.had === 1 ? "was" : "were"} left as ${r.had === 1 ? "it was" : "they were"}.`;
  if (r.other_vault && !r.adopted) text += ` What vault ${r.other_vault} sealed opens here now.`;
  return text;
}

/// Chooses where to bring things in from: a folder (a backup, a copy of Documents › Superteem Voice, or
/// Voice's data folder) or a vault file.
async function chooseBringIn(folder) {
  try {
    const path = await window.__TAURI__.dialog.open(folder
      ? { directory: true, title: "Choose a backup, or a Superteem Voice folder", defaultPath: vault?.backup_dir }
      : { title: "Choose a vault file", filters: [{ name: "Vault", extensions: ["db"] }] });
    if (path) showVaultBringIn({ path, folder });
  } catch (e) {
    micToast(String(e));
  }
}

/// Opens what was chosen with its words or password, then either takes it on as this computer's vault (a
/// new computer) or adds what's in it to this one.
function showVaultBringIn({ path, folder, adopt = false }) {
  vaultSheet(adopt ? "Open your vault on this computer" : "Bring in from a backup", (dialog, body) => {
    const [box, count] = vaultWordsBox("That vault's 24 recovery words");
    const password = el("input", { type: "password", autocomplete: "off", "aria-label": "That vault's password" });
    const run = (command, button) => async () => {
      const result = await vaultTry(button, body, () => invoke(command, { path, words: box.value, password: password.value }));
      if (!result) return;
      dialog.close();
      await vaultChanged();
      vaultSheet("Done", (done, inner) => {
        vaultAdd(inner, el("p", { class: "hint" }, broughtText(result)),
          result.failed?.length ? el("div", { class: "notice" }, `Couldn't bring in: ${result.failed.join("; ")}.`) : null,
          result.adopted ? el("p", { class: "hint" }, "Your recovery words for that vault keep working, and this computer opens it by itself from now on.") : null,
          !result.adopted && result.notes + result.books > 0 ? el("p", { class: "hint" }, "This vault's own recovery words are unchanged.") : null,
          el("div", { class: "actions" }, el("button", { class: "primary pill", onclick: () => done.close() }, "OK")));
      });
    };
    const take = el("button", { class: adopt ? "primary pill" : "pill" }, "Use it on this computer");
    const add = el("button", { class: adopt ? "pill" : "primary pill" }, "Add what's in it here");
    take.addEventListener("click", run("vault_adopt", take));
    add.addEventListener("click", run(folder ? "vault_bring_in_folder" : "vault_bring_in", add));
    const fresh = (vault?.own_notes ?? 0) === 0;
    vaultAdd(body, 
      el("p", { class: "vault-path" }, path),
      el("p", { class: "hint" }, folder
        ? "Voice looks in this folder for the newest copy of a vault, notes and books kept before the vault, and meetings. Nothing in the folder is changed."
        : "A vault file: a daily copy from Vault backups, a vault set aside, or one from another computer."),
      el("label", { class: "field" }, el("span", {}, "That vault's recovery words", el("small", {}, "Leave empty for a copy of this vault, or for notes kept before the vault.")), box, count),
      el("details", { class: "vault-more" }, el("summary", {}, "Use its vault password instead"),
        el("label", { class: "field" }, el("span", {}, "That vault's password", el("small", {}, "Works for a copy from a Vault backups folder, which keeps the key file the password needs.")), password)),
      vault?.sealed ? el("div", { class: "vault-choice" },
        el("div", {}, el("strong", {}, "Use it on this computer"), el("small", {}, fresh
          ? "For a new computer. That vault becomes this computer's, and its recovery words keep working."
          : "That vault becomes this computer's, and its words keep working. Everything here now is added to it, and this vault is set aside, not deleted.")),
        el("div", {}, el("strong", {}, "Add what's in it here"), el("small", {}, "Its notes, books and meetings join this vault, which keeps its own recovery words. Anything already here is left as it is."))) : null,
      el("div", { class: "actions" }, el("button", { class: "pill", onclick: () => dialog.close() }, "Cancel"), el("span", { class: "grow" }), vault?.sealed ? take : null, add));
    box.focus();
  }, { wide: true });
}

/// Meetings copied from another computer, sealed by its vault, open here once its words are added.
function showVaultAddWords() {
  vaultSheet("Open meetings from another vault", (dialog, body) => {
    const [box, count] = vaultWordsBox("The other vault's 24 recovery words");
    const add = el("button", { class: "primary pill" }, "Add");
    add.addEventListener("click", async () => {
      const id = await vaultTry(add, body, () => invoke("vault_add_words", { words: box.value }));
      if (!id) return;
      dialog.close();
      await vaultChanged();
      micToast(`Meetings and files sealed by vault ${id} open here now.`);
    });
    vaultAdd(body, el("p", { class: "hint" }, "Meetings shown as “Locked meeting (from another vault)” were sealed on another computer, or by a vault you replaced. Type that vault's words and they open here. The words are kept inside this vault, encrypted."),
      el("label", { class: "field" }, el("span", {}, "Recovery words"), box, count),
      el("div", { class: "actions" }, el("button", { class: "pill", onclick: () => dialog.close() }, "Cancel"), add));
    box.focus();
  });
}

async function chooseBackupFolder(page) {
  try {
    const path = await window.__TAURI__.dialog.open({ directory: true, title: "Keep a daily copy of the vault in this folder", defaultPath: vault?.backup_dir });
    if (!path) return;
    vault = await invoke("vault_set_backup_dir", { folder: path });
    showVaultSettings();
    micToast("A copy is in that folder now, and one goes there each day.");
  } catch (e) {
    showError(page, e);
  }
}

// ---------- Settings › Vault ----------

function vaultSummary() {
  if (!vault) return "";
  if (vault.state === "locked") return "Locked";
  if (vault.state === "damaged") return "Needs bringing back";
  if (!vault.sealed) return "Not encrypted on this device yet";
  return vault.words_saved ? "Encrypted · recovery words saved" : "Encrypted · recovery words not saved";
}

async function showVaultSettings() {
  leave();
  showPage();
  view = { kind: "vault" };
  refreshItems();
  await refreshVault();
  const page = el("div", { class: "page" }, el("button", { class: "link back-link", onclick: () => showSettings() }, "‹ Settings"), el("h1", {}, "Vault"));
  const row = (label, detail, ...actions) => el("div", { class: "row" }, el("span", { class: "label" }, label, detail ? el("small", {}, detail) : null), ...actions.filter(Boolean));
  const button = (text, onclick, kind = "pill") => el("button", { class: kind, onclick }, text);
  if (!vault) {
    vaultAdd(page, el("div", { class: "error", role: "alert" }, "The vault's state couldn't be read."));
    main.replaceChildren(page);
    return;
  }
  if (vault.state !== "open") {
    const damaged = vault.state === "damaged";
    vaultAdd(page, el("div", { class: "notice" }, el("strong", {}, damaged ? "The vault can't be read. " : "Your notes, books and meetings are locked. "),
      damaged ? "An earlier copy can take its place." : "This computer can't open the vault by itself; your recovery words or password can."),
      el("div", { class: "actions" }, damaged ? button("Bring back a copy", () => showVaultCopies(), "primary pill") : button("Open them", () => showVaultUnlock(), "primary pill")));
    main.replaceChildren(page);
    return;
  }
  const counts = [vaultPlural(vault.own_notes, "note"), vaultPlural(vault.books, "book")].join(" · ");
  if (!vault.sealed) {
    vaultAdd(page, el("p", { class: "hint" }, `Your notes, books and what's written about them (${counts}) are kept in one database on this ${isPhone() ? "phone" : "computer"}. Encryption comes to Windows first; here it isn't encrypted yet.`));
  } else {
    vaultAdd(page, 
      el("p", { class: "hint" }, `Your notes, books, bookmarks, chats, analyses and translations (${counts}) are in one encrypted database. Meetings, and the pictures and files in notes and books, are sealed with the same key. This computer opens it by itself, for your Windows account only. Copied anywhere else, none of it can be read without one of the ways back in below.`),
      el("p", { class: "hint vault-id-line" }, "Vault ID ", el("strong", {}, vault.id), " · your recovery kit and backup copies carry the same ID."),
      vault.sealing ? el("div", { class: "notice" }, `Sealing what was recorded before: ${vaultPlural(vault.sealing, "file")} to go. It carries on in the background and picks up again next time Voice starts.`) : null);

    // Ways back in: each with whether it's ready.
    const sixMonths = 183 * 24 * 3600;
    const stale = vault.words_saved && Date.now() / 1000 - vault.words_checked > sixMonths;
    const backupAge = vault.backup_latest ? Date.now() / 1000 - vault.backup_latest : Infinity;
    const ready = [true, !!vault.words_saved, vault.password, backupAge < 3 * 24 * 3600].filter(Boolean).length;
    const mark = (ok) => el("span", { class: "vault-mark" + (ok ? " ok" : ""), "aria-label": ok ? "Ready" : "Not set up" }, ok ? "✓" : "!");
    const way = (ok, label, detail, ...actions) => el("div", { class: "row vault-way" }, mark(ok), el("span", { class: "label" }, label, el("small", {}, detail)), ...actions.filter(Boolean));
    vaultAdd(page, el("h2", {}, "Ways back in"),
      el("p", { class: "hint" }, `${ready} of 4 ready. Any one of them opens your vault; the more you have, the less one lost matters.`),
      el("div", { class: "group" },
        way(true, "This Windows account", "Opens the vault by itself, day to day. Lost if Windows is reinstalled or this computer is replaced."),
        way(!!vault.words_saved, "Recovery words", vault.words_saved
          ? `Saved ${vaultDate(vault.words_saved)} · last checked ${vaultDate(vault.words_checked)}${stale ? " · worth checking your copy again" : ""}`
          : "24 words that open the vault on any computer. Not saved yet.",
          vault.words_saved ? button("Check my copy", () => showVaultCheck()) : null,
          button(vault.words_saved ? "Show" : "Show and save", () => showVaultWords(), vault.words_saved ? "pill" : "primary pill")),
        way(vault.password, "Vault password", vault.password ? `Set${vault.hint ? ` · hint: ${vault.hint}` : ""}` : "Optional: a password you choose, with a hint you write for yourself.",
          vault.password ? button("Remove", async (e) => {
            const status = await vaultTry(e.currentTarget, page, () => invoke("vault_set_password", { password: "", hint: "" }));
            if (status) { vault = status; showVaultSettings(); }
          }) : null,
          button(vault.password ? "Change" : "Set a password", () => showVaultPassword())),
        way(backupAge < 3 * 24 * 3600, "A copy that outlives this computer",
          (vault.backup_latest ? `Latest ${vaultWhen(vault.backup_latest)}` : "None yet") + ` · ${vault.backup_dir}`
            + (vault.backup_reachable ? "" : " · that folder can't be reached now") + (vault.backup_chosen ? "" : " · keep Documents backed up or synced, or choose a folder that is"),
          button("Choose a folder…", () => chooseBackupFolder(page)), button("Open", () => invoke("vault_open_backups").catch((e) => showError(page, e))))));
    if (!vault.words_saved) {
      vaultAdd(page, el("div", { class: "notice" }, el("strong", {}, "Save your recovery words. "),
        "If Windows is reinstalled or this computer is replaced, they're the way back into your notes, books and meetings."));
    }
  }
  const latest = vault.snapshots[0];
  const copyNow = el("button", { class: "pill" }, "Copy now");
  copyNow.addEventListener("click", async () => {
    const status = await vaultTry(copyNow, page, () => invoke("vault_copy_now"));
    if (status) { vault = status; showVaultSettings(); micToast("Copied, here and to your backup folder."); }
  });
  vaultAdd(page, el("h2", {}, "Copies on this computer"), el("div", { class: "group" },
    row(latest ? `Latest ${vaultWhen(latest.at)}` : "No copy yet", latest ? `${vaultPlural(vault.snapshots.length, "copy", "copies")} kept: the latest dozen, one a day for a fortnight, one a month for a year.` : "One is made within the hour once there are notes.",
      copyNow, vault.snapshots.length ? button("Bring back a copy…", () => showVaultCopies()) : null)));
  if (isDesktop()) {
    vaultAdd(page, el("h2", {}, "Bring in"), el("div", { class: "group" },
      row("From a backup or another computer", "A Vault backups folder, a copy of Documents › Superteem Voice, or Voice's data folder from before the vault. Brings notes, books and meetings.",
        button("Choose a folder…", () => chooseBringIn(true))),
      row("From a vault file", "A single daily copy, or a vault that was set aside.", button("Choose a file…", () => chooseBringIn(false))),
      vault.sealed ? row("Meetings from another vault", vault.keyring ? `This vault also opens what ${vaultPlural(vault.keyring, "other vault")} sealed.` : "Open meetings sealed on another computer, with that vault's words.",
        button("Add its words…", () => showVaultAddWords())) : null));
  }
  vaultAdd(page, el("h2", {}, "Where it is"), el("div", { class: "group" }, row("The vault", vault.folder), row("Meetings", "Documents › Superteem Voice, sealed beside the vault's daily copies")));
  main.replaceChildren(page);
}
