const invoke = window.__TAURI__.core.invoke;

const sourceLabels = {
  humble_bundle: "Humble Bundle",
  packt: "Packt",
  manning: "Manning",
  kindle: "Kindle",
  manual: "Manual",
};

function splitList(value) {
  return value
    .split(",")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

function nonEmpty(value) {
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

// --- Tabs ---

document.querySelectorAll(".tab-btn").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".tab-btn").forEach((b) => b.classList.remove("active"));
    document.querySelectorAll(".view").forEach((v) => v.classList.remove("active"));
    btn.classList.add("active");
    document.getElementById(`view-${btn.dataset.view}`).classList.add("active");
    if (btn.dataset.view === "settings") loadSettings();
  });
});

// --- Books list ---

async function loadBooks() {
  const source = document.getElementById("source-filter").value || null;
  const books = await invoke("list_books", { source });
  const tbody = document.getElementById("book-rows");
  tbody.innerHTML = "";
  document.getElementById("books-empty").classList.toggle("hidden", books.length > 0);

  for (const book of books) {
    const tr = document.createElement("tr");
    tr.innerHTML = `
      <td>${escapeHtml(book.title)}</td>
      <td>${escapeHtml(book.authors.join(", "))}</td>
      <td>${sourceLabels[book.source] || book.source}</td>
      <td>${escapeHtml(book.formats.join(", "))}</td>
      <td>${escapeHtml(book.isbn || "")}</td>
      <td class="row-actions">
        <button data-action="edit" data-id="${book.id}">Edit</button>
        <button data-action="remove" data-id="${book.id}">Remove</button>
      </td>
    `;
    tbody.appendChild(tr);
  }

  tbody.querySelectorAll("button[data-action='edit']").forEach((btn) => {
    btn.addEventListener("click", () => openEditModal(Number(btn.dataset.id)));
  });
  tbody.querySelectorAll("button[data-action='remove']").forEach((btn) => {
    btn.addEventListener("click", () => removeBook(Number(btn.dataset.id)));
  });

  await loadStats();
}

async function loadStats() {
  const stats = await invoke("stats");
  const total = stats.reduce((sum, [, count]) => sum + count, 0);
  const parts = stats.map(([source, count]) => `${sourceLabels[source] || source}: ${count}`);
  document.getElementById("stats-line").textContent = stats.length
    ? `${total} total — ${parts.join(", ")}`
    : "no books yet";
}

async function removeBook(id) {
  if (!confirm("Remove this book from your library?")) return;
  await invoke("remove_book", { id });
  await loadBooks();
}

document.getElementById("source-filter").addEventListener("change", loadBooks);

function escapeHtml(s) {
  const div = document.createElement("div");
  div.textContent = s;
  return div.innerHTML;
}

// --- Add/Edit modal ---

const modal = document.getElementById("book-modal");
let editingId = null;

document.getElementById("add-book-btn").addEventListener("click", () => {
  editingId = null;
  document.getElementById("book-modal-title").textContent = "Add book";
  document.getElementById("field-title").value = "";
  document.getElementById("field-authors").value = "";
  document.getElementById("field-isbn").value = "";
  document.getElementById("field-formats").value = "";
  document.getElementById("modal-warnings").innerHTML = "";
  modal.classList.remove("hidden");
});

async function openEditModal(id) {
  const book = await invoke("get_book", { id });
  if (!book) return;
  editingId = id;
  document.getElementById("book-modal-title").textContent = "Edit book";
  document.getElementById("field-title").value = book.title;
  document.getElementById("field-authors").value = book.authors.join(", ");
  document.getElementById("field-isbn").value = book.isbn || "";
  document.getElementById("field-formats").value = book.formats.join(", ");
  document.getElementById("modal-warnings").innerHTML = "";
  modal.classList.remove("hidden");
}

document.getElementById("modal-cancel").addEventListener("click", () => modal.classList.add("hidden"));

document.getElementById("modal-save").addEventListener("click", async () => {
  const title = document.getElementById("field-title").value.trim();
  if (!title) return;
  const authors = splitList(document.getElementById("field-authors").value);
  const isbn = nonEmpty(document.getElementById("field-isbn").value);
  const formats = splitList(document.getElementById("field-formats").value);

  if (editingId === null) {
    const result = await invoke("add_book", { title, authors, isbn, formats });
    renderWarnings(result.warnings);
    if (result.warnings.length === 0) {
      modal.classList.add("hidden");
    }
  } else {
    await invoke("update_book", { id: editingId, title, authors, isbn, formats });
    modal.classList.add("hidden");
  }
  await loadBooks();
});

function renderWarnings(warnings) {
  const el = document.getElementById("modal-warnings");
  if (!warnings || warnings.length === 0) {
    el.innerHTML = "";
    return;
  }
  el.innerHTML =
    `<p class="hint">Possible duplicates — saved anyway, review if needed:</p>` +
    warnings
      .map(
        (m) =>
          `<div class="match-card">${escapeHtml(m.book.title)} (${sourceLabels[m.book.source] || m.book.source}) — ${(m.confidence * 100).toFixed(0)}% match, ${escapeHtml(m.reason)}</div>`
      )
      .join("");
}

// --- Check before buying ---

document.getElementById("check-btn").addEventListener("click", runCheck);
document.getElementById("check-query").addEventListener("keydown", (e) => {
  if (e.key === "Enter") runCheck();
});

async function runCheck() {
  const query = document.getElementById("check-query").value.trim();
  const results = document.getElementById("check-results");
  if (!query) {
    results.innerHTML = "";
    return;
  }
  const { strong, weak } = await invoke("check_duplicates", { query });

  if (strong.length === 0 && weak.length === 0) {
    results.innerHTML = `<p class="hint">No matches found — looks safe to buy.</p>`;
    return;
  }

  let html = "";
  if (strong.length) {
    html += "<h3>Likely matches</h3>";
    html += strong.map((m) => matchCard(m, false)).join("");
  }
  if (weak.length) {
    html += "<h3>Weaker matches (review manually)</h3>";
    html += weak.map((m) => matchCard(m, true)).join("");
  }
  results.innerHTML = html;
}

function matchCard(m, isWeak) {
  return `<div class="match-card${isWeak ? " weak" : ""}">${escapeHtml(m.book.title)} (${sourceLabels[m.book.source] || m.book.source}) — ${(m.confidence * 100).toFixed(0)}% match, ${escapeHtml(m.reason)}</div>`;
}

// --- Import ---

document.querySelectorAll(".import-btn").forEach((btn) => {
  btn.addEventListener("click", async () => {
    const source = btn.dataset.source;
    const file = source === "kindle" ? document.getElementById("kindle-path").value.trim() : null;
    if (source === "kindle" && !file) {
      alert("Enter a path to a Kindle CSV file first.");
      return;
    }
    btn.disabled = true;
    btn.textContent = "Importing...";
    const results = document.getElementById("import-results");
    try {
      const summary = await invoke("import_source", { source, file });
      let html = `<div class="match-card">${sourceLabels[summary.source] || summary.source}: ${summary.new_count} new, ${summary.updated_count} updated</div>`;
      html += summary.warnings.map((w) => `<div class="match-card weak">${escapeHtml(w)}</div>`).join("");
      results.innerHTML = html + results.innerHTML;
      await loadBooks();
    } catch (e) {
      results.innerHTML = `<div class="match-card">Error: ${escapeHtml(String(e))}</div>` + results.innerHTML;
    } finally {
      btn.disabled = false;
      btn.textContent = "Import";
    }
  });
});

// --- Settings ---

async function loadSettings() {
  const status = await invoke("get_config_status");
  setPill("status-humble", status.humble_cookie_set);
  setPill("status-packt", status.packt_token_set);
  setPill("status-manning", status.manning_cookies_set);
  document.getElementById("db-path").textContent = status.db_path;
}

function setPill(id, isSet) {
  const el = document.getElementById(id);
  el.textContent = isSet ? "configured" : "not set";
  el.classList.toggle("set", isSet);
}

document.getElementById("save-settings-btn").addEventListener("click", async () => {
  const fields = [
    ["cred-humble", "humble_cookie"],
    ["cred-packt", "packt_token"],
    ["cred-manning", "manning_cookies"],
  ];
  let savedAny = false;
  for (const [inputId, field] of fields) {
    const input = document.getElementById(inputId);
    const value = input.value.trim();
    if (value) {
      await invoke("set_credential", { field, value });
      input.value = "";
      savedAny = true;
    }
  }
  document.getElementById("settings-status").textContent = savedAny ? "Saved." : "Nothing to save.";
  await loadSettings();
});

document.querySelectorAll(".capture-btn").forEach((btn) => {
  btn.addEventListener("click", async () => {
    const source = btn.dataset.capture;
    const label = sourceLabels[source] || source;
    const original = btn.textContent;
    const status = document.getElementById("settings-status");
    btn.disabled = true;
    btn.textContent = "Waiting for login\u2026";
    status.textContent = `A login window opened for ${label} \u2014 log in there and it finishes automatically.`;
    try {
      const result = await invoke("capture_credential", { source });
      status.textContent = result.cancelled
        ? `Cancelled \u2014 the ${label} login window was closed before capture finished.`
        : `${label} credential captured and saved.`;
      await loadSettings();
    } catch (e) {
      status.textContent = `Error capturing ${label}: ${e}`;
    } finally {
      btn.disabled = false;
      btn.textContent = original;
    }
  });
});

// --- Init ---

loadBooks();
