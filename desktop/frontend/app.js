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

// --- View switching (tabs + book detail, which isn't a tab) ---

function showView(id) {
  document.querySelectorAll(".view").forEach((v) => v.classList.remove("active"));
  document.getElementById(`view-${id}`).classList.add("active");
}

document.querySelectorAll(".tab-btn").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".tab-btn").forEach((b) => b.classList.remove("active"));
    btn.classList.add("active");
    showView(btn.dataset.view);
    if (btn.dataset.view === "settings") loadSettings();
  });
});

// --- Books list ---

let currentBooks = [];
let currentViewMode = "list";

async function loadBooks() {
  const source = document.getElementById("source-filter").value || null;
  currentBooks = await invoke("list_books", { source });
  document.getElementById("books-empty").classList.toggle("hidden", currentBooks.length > 0);
  renderBooks();
  await loadStats();
}

function renderBooks() {
  if (currentViewMode === "grid") {
    renderGrid(currentBooks);
  } else {
    renderList(currentBooks);
  }
}

function renderList(books) {
  const tbody = document.getElementById("book-rows");
  tbody.innerHTML = "";

  for (const book of books) {
    const tr = document.createElement("tr");
    const dupeSources = book.duplicate_sources || [];
    const dupeBadge = dupeSources.length
      ? `<span class="dupe-badge" title="Also owned via ${escapeHtml(dupeSources.map((s) => sourceLabels[s] || s).join(", "))}">also: ${escapeHtml(dupeSources.map((s) => sourceLabels[s] || s).join(", "))}</span>`
      : "";
    tr.innerHTML = `
      <td class="cover-cell">${
        book.cover_url
          ? `<img class="cover-thumb" src="${escapeHtml(book.cover_url)}" alt="" loading="lazy" onerror="this.remove()" />`
          : ""
      }</td>
      <td><button type="button" class="link-btn book-title-link" data-id="${book.id}">${escapeHtml(book.title)}</button>${dupeBadge}</td>
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

  tbody.querySelectorAll("button.book-title-link").forEach((btn) => {
    btn.addEventListener("click", () => openBookDetail(Number(btn.dataset.id)));
  });
  tbody.querySelectorAll("button[data-action='edit']").forEach((btn) => {
    btn.addEventListener("click", () => openEditModal(Number(btn.dataset.id)));
  });
  tbody.querySelectorAll("button[data-action='remove']").forEach((btn) => {
    btn.addEventListener("click", () => removeBook(Number(btn.dataset.id)));
  });
}

// Thumbnail-only: no title/author/source text on the card itself, just the
// cover (or a compact title fallback for books with no cover_url) -- the
// full title is still available as a native tooltip, and clicking opens
// the book's detail page.
function renderGrid(books) {
  const grid = document.getElementById("book-grid");
  grid.innerHTML = "";

  for (const book of books) {
    const cell = document.createElement("button");
    cell.type = "button";
    cell.className = "grid-cell";
    cell.title = book.title;
    cell.dataset.id = book.id;

    if (book.cover_url) {
      const img = document.createElement("img");
      img.src = book.cover_url;
      img.alt = "";
      img.loading = "lazy";
      img.addEventListener("error", () => {
        cell.innerHTML = "";
        cell.appendChild(gridCellFallback(book.title));
      });
      img.addEventListener("load", () => {
        // Some sources (e.g. Humble Bundle, which only exposes a 70x70
        // subproduct icon -- see core/src/sources/humble.rs) hand back an
        // image too small to ever fill a grid cell without visible upscale
        // blur: .book-grid's CSS floor is a 120px-wide cell (minmax(120px,
        // 1fr)), so anything narrower than that is guaranteed to be
        // stretched on every window size. Prefer a crisp, readable title
        // card over a soft, unreadable one.
        if (img.naturalWidth < 120 || img.naturalHeight < 120) {
          cell.innerHTML = "";
          cell.appendChild(gridCellFallback(book.title));
        }
      });
      cell.appendChild(img);
    } else {
      cell.appendChild(gridCellFallback(book.title));
    }

    cell.addEventListener("click", () => openBookDetail(book.id));
    grid.appendChild(cell);
  }
}

function gridCellFallback(title) {
  const span = document.createElement("span");
  span.className = "grid-cell-fallback";
  span.textContent = title;
  return span;
}

function setViewMode(mode) {
  currentViewMode = mode;
  document.querySelectorAll(".view-toggle .toggle-btn").forEach((btn) => {
    btn.classList.toggle("active", btn.dataset.viewMode === mode);
  });
  document.getElementById("book-table").classList.toggle("hidden", mode !== "list");
  document.getElementById("book-grid").classList.toggle("hidden", mode !== "grid");
  renderBooks();
}

document.querySelectorAll(".view-toggle .toggle-btn").forEach((btn) => {
  btn.addEventListener("click", () => setViewMode(btn.dataset.viewMode));
});

async function loadStats() {
  const stats = await invoke("stats");
  const total = stats.reduce((sum, [, count]) => sum + count, 0);
  const parts = stats.map(([source, count]) => `${sourceLabels[source] || source}: ${count}`);
  document.getElementById("stats-line").textContent = stats.length
    ? `${total} total — ${parts.join(", ")}`
    : "no books yet";
}

async function removeBook(id) {
  if (!confirm("Remove this book from your library?")) return false;
  await invoke("remove_book", { id });
  await loadBooks();
  return true;
}

document.getElementById("source-filter").addEventListener("change", loadBooks);

// --- Metadata enrichment ---

document.getElementById("enrich-btn").addEventListener("click", async () => {
  const btn = document.getElementById("enrich-btn");
  const status = document.getElementById("enrich-status");
  btn.disabled = true;
  btn.textContent = "Enriching…";
  status.textContent = "";
  try {
    const summary = await invoke("enrich_metadata");
    status.textContent =
      summary.checked === 0
        ? "nothing missing authors, an ISBN, or a cover"
        : `checked ${summary.checked}, updated ${summary.updated}, no match ${summary.not_found}, errors ${summary.errors}`;
    await loadBooks();
  } catch (e) {
    status.textContent = `enrichment failed: ${String(e)}`;
  } finally {
    btn.disabled = false;
    btn.textContent = "Fill missing metadata";
  }
});

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
  document.getElementById("field-cover-url").value = "";
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
  document.getElementById("field-cover-url").value = book.cover_url || "";
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
  const coverUrl = nonEmpty(document.getElementById("field-cover-url").value);

  if (editingId === null) {
    const result = await invoke("add_book", { title, authors, isbn, formats, coverUrl });
    renderWarnings(result.warnings);
    if (result.warnings.length === 0) {
      modal.classList.add("hidden");
    }
  } else {
    await invoke("update_book", { id: editingId, title, authors, isbn, formats, coverUrl });
    modal.classList.add("hidden");
  }
  await loadBooks();
  if (editingId !== null && document.getElementById("view-book").classList.contains("active")) {
    await openBookDetail(editingId);
  }
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

// --- Book detail page ---

let currentDetailId = null;

async function openBookDetail(id) {
  const detail = await invoke("get_book_detail", { id });
  if (!detail) return;
  currentDetailId = id;

  document.getElementById("book-detail-title").textContent = detail.title;
  document.getElementById("book-detail-authors").textContent = detail.authors.join(", ");
  document.getElementById("book-detail-source").textContent = sourceLabels[detail.source] || detail.source;
  document.getElementById("book-detail-isbn").textContent = detail.isbn || "\u2014";
  document.getElementById("book-detail-formats").textContent = detail.formats.join(", ") || "\u2014";
  document.getElementById("book-detail-acquired").textContent = detail.acquired_at || "\u2014";
  document.getElementById("book-detail-source-id").textContent = detail.source_id || "\u2014";

  const img = document.getElementById("book-detail-cover-img");
  const fallback = document.getElementById("book-detail-cover-fallback");
  if (detail.cover_url) {
    img.src = detail.cover_url;
    img.classList.remove("hidden");
    fallback.classList.add("hidden");
    img.onerror = () => {
      img.classList.add("hidden");
      fallback.textContent = detail.title;
      fallback.classList.remove("hidden");
    };
  } else {
    img.classList.add("hidden");
    fallback.textContent = detail.title;
    fallback.classList.remove("hidden");
  }

  const dupes = document.getElementById("book-detail-duplicates");
  dupes.innerHTML = detail.duplicates.length
    ? detail.duplicates.map((m) => matchCard(m, m.confidence < 0.9)).join("")
    : `<p class="hint">No copies of this found from another source.</p>`;

  const rawWrap = document.getElementById("book-detail-raw-wrap");
  rawWrap.open = false;
  const raw = document.getElementById("book-detail-raw");
  if (detail.raw_json) {
    try {
      raw.textContent = JSON.stringify(JSON.parse(detail.raw_json), null, 2);
    } catch {
      raw.textContent = detail.raw_json;
    }
    rawWrap.classList.remove("hidden");
  } else {
    rawWrap.classList.add("hidden");
  }

  showView("book");
}

document.getElementById("book-detail-back").addEventListener("click", () => showView("books"));
document.getElementById("book-detail-edit").addEventListener("click", () => openEditModal(currentDetailId));
document.getElementById("book-detail-remove").addEventListener("click", async () => {
  if (currentDetailId === null) return;
  if (await removeBook(currentDetailId)) showView("books");
});

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

document.getElementById("bundle-check-btn").addEventListener("click", runBundleCheck);
document.getElementById("bundle-url").addEventListener("keydown", (e) => {
  if (e.key === "Enter") runBundleCheck();
});
document.getElementById("active-bundles-check-btn").addEventListener("click", runActiveBundlesCheck);
let lastBundleCheck = null; // { type: "single", result } | { type: "active", results } | null
let bundleExcludeTerms = [];

function excludeFictionChecked() {
  return document.getElementById("exclude-fiction").checked;
}

// Mirrors `library_core::sources::humble::matches_excluded_bundle` --
// case-insensitive substring match of any configured term against the
// bundle's own name. Applied here (against the cached fetch) rather than
// server-side so adding/removing a term in Settings re-filters
// "Check against current bundles" results immediately instead of
// requiring a full re-fetch of every bundle.
function matchesExcludedBundle(bundleName, terms) {
  const lower = bundleName.toLowerCase();
  return terms.some((term) => term.trim() !== "" && lower.includes(term.trim().toLowerCase()));
}

// Whole-bundle exclusion only -- individual book titles within a kept
// bundle are never filtered out, just like `matchesExcludedBundle`.
// `is_fiction_or_comic` is decided once server-side from the bundle's own
// name (`BundleCheckResult.is_fiction_or_comic`) and shipped with every
// fetch, so toggling the checkbox re-filters the cached result set below
// instead of re-invoking the Tauri command and re-fetching every bundle.
function isBundleExcluded(result, excludeFiction) {
  if (result.error) return false;
  if (matchesExcludedBundle(result.bundle_name, bundleExcludeTerms)) return true;
  return excludeFiction && result.is_fiction_or_comic;
}

async function runBundleCheck() {
  const url = document.getElementById("bundle-url").value.trim();
  const results = document.getElementById("bundle-check-results");
  if (!url) {
    lastBundleCheck = null;
    results.innerHTML = "";
    return;
  }
  const btn = document.getElementById("bundle-check-btn");
  btn.disabled = true;
  btn.textContent = "Checking\u2026";
  results.innerHTML = "";
  try {
    const result = await invoke("check_bundle_url", { url });
    lastBundleCheck = { type: "single", result };
    renderBundleCheckResults();
  } catch (e) {
    lastBundleCheck = null;
    results.innerHTML = `<div class="match-card">Error: ${escapeHtml(String(e))}</div>`;
  } finally {
    btn.disabled = false;
    btn.textContent = "Check bundle";
  }
}

async function runActiveBundlesCheck() {
  const results = document.getElementById("bundle-check-results");
  const btn = document.getElementById("active-bundles-check-btn");
  btn.disabled = true;
  btn.textContent = "Checking current bundles\u2026";
  results.innerHTML = `<p class="hint">Fetching every bundle currently on humblebundle.com/books\u2026 this checks each one in turn, so it can take a little while.</p>`;
  try {
    const bundleResults = await invoke("check_active_bundles");
    lastBundleCheck = { type: "active", results: bundleResults };
    renderBundleCheckResults();
  } catch (e) {
    lastBundleCheck = null;
    results.innerHTML = `<div class="match-card">Error: ${escapeHtml(String(e))}</div>`;
  } finally {
    btn.disabled = false;
    btn.textContent = "Check against current bundles";
  }
}

function renderBundleCheckResults() {
  const results = document.getElementById("bundle-check-results");
  if (!lastBundleCheck) return;
  if (lastBundleCheck.type === "single") {
    results.innerHTML = bundleResultBlock(lastBundleCheck.result);
  } else {
    const excludeFiction = excludeFictionChecked();
    const bundleResults = lastBundleCheck.results.filter(
      (r) => !isBundleExcluded(r, excludeFiction)
    );
    const excludedCount = lastBundleCheck.results.length - bundleResults.length;
    const okResults = bundleResults.filter((r) => !r.error);
    const ownedCount = okResults.reduce(
      (sum, r) => sum + r.items.filter((item) => item.strong.length > 0).length,
      0
    );
    const excludedNote = excludedCount
      ? `, ${excludedCount} excluded by your bundle-exclude terms/fiction filter`
      : "";
    let html = `<p class="hint">${bundleResults.length} bundles checked (${okResults.length} succeeded${excludedNote}) \u2014 ${ownedCount} books across them look like ones you already own.</p>`;
    html += bundleResults.map(bundleResultBlock).join("");
    results.innerHTML = html;
  }
  wireBundleCheckLinks();
}

document.getElementById("exclude-fiction").addEventListener("change", renderBundleCheckResults);

function bundleResultBlock(result) {
  const name = escapeHtml(result.error ? result.url : result.bundle_name);
  const openBtn = `<button type="button" class="link-btn inline bundle-open-btn" data-url="${escapeHtml(result.url)}" title="Open bundle page" aria-label="Open bundle page">\u2197</button>`;
  const header = `<h3 class="bundle-group-header"><button type="button" class="bundle-toggle-btn" aria-expanded="true"><span class="chevron">\u25be</span> ${name}</button>${openBtn}</h3>`;
  if (result.error) {
    return `<div class="bundle-group">${header}<div class="bundle-group-body"><div class="match-card">Error: ${escapeHtml(result.error)}</div></div></div>`;
  }
  const items = result.items;
  const ownedCount = items.filter((item) => item.strong.length > 0).length;
  let body = `<p class="hint">${ownedCount} of ${items.length} books look like ones you already own.</p>`;
  body += items.map(bundleItemCard).join("");
  return `<div class="bundle-group">${header}<div class="bundle-group-body">${body}</div></div>`;
}

function matchLinks(matches) {
  return matches
    .map(
      (m) =>
        `<button type="button" class="link-btn inline book-link" data-id="${m.book.id}">${escapeHtml(m.book.title)} (${escapeHtml(sourceLabels[m.book.source] || m.book.source)}, ${(m.confidence * 100).toFixed(0)}% match)</button>`
    )
    .join("; ");
}

function bundleItemCard(item) {
  const byline = item.authors.length ? ` by ${escapeHtml(item.authors.join(", "))}` : "";
  const heading = `${escapeHtml(item.title)}${byline}`;
  if (item.strong.length) {
    return `<div class="match-card">\u2713 ${heading} \u2014 already in your library (${matchLinks(item.strong)})</div>`;
  }
  if (item.weak.length) {
    return `<div class="match-card weak">? ${heading} \u2014 possible match, review manually (${matchLinks(item.weak)})</div>`;
  }
  return `<div class="match-card new">${heading} \u2014 new to you</div>`;
}

function wireBundleCheckLinks() {
  document.querySelectorAll("#bundle-check-results .book-link").forEach((btn) => {
    btn.addEventListener("click", () => openBookDetail(Number(btn.dataset.id)));
  });
  document.querySelectorAll("#bundle-check-results .bundle-open-btn").forEach((btn) => {
    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      invoke("open_url", { url: btn.dataset.url });
    });
  });
  document.querySelectorAll("#bundle-check-results .bundle-toggle-btn").forEach((btn) => {
    btn.addEventListener("click", () => {
      const group = btn.closest(".bundle-group");
      const collapsed = group.classList.toggle("collapsed");
      btn.setAttribute("aria-expanded", String(!collapsed));
    });
  });
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
  setPill("status-packt", status.packt_cookies_set);
  setPill("status-manning", status.manning_cookies_set);
  document.getElementById("db-path").textContent = status.db_path;
  renderBundleExcludeTerms(status.bundle_exclude_terms);
}

function renderBundleExcludeTerms(terms) {
  bundleExcludeTerms = terms;
  const container = document.getElementById("bundle-exclude-terms");
  if (!terms.length) {
    container.innerHTML = `<p class="hint">No exclude terms yet \u2014 every current bundle will be checked.</p>`;
  } else {
    container.innerHTML = terms
      .map(
        (term) =>
          `<span class="term-chip">${escapeHtml(term)}<button type="button" class="remove-term-btn" data-term="${escapeHtml(term)}" title="Remove">&times;</button></span>`
      )
      .join("");
    container.querySelectorAll(".remove-term-btn").forEach((btn) => {
      btn.addEventListener("click", async () => {
        const remaining = await invoke("remove_bundle_exclude_term", { term: btn.dataset.term });
        renderBundleExcludeTerms(remaining);
      });
    });
  }
  // Re-filter any already-fetched "Check against current bundles" results
  // against the updated term list instead of requiring a re-fetch.
  renderBundleCheckResults();
}

async function addBundleExcludeTerm() {
  const input = document.getElementById("bundle-exclude-input");
  const term = input.value.trim();
  if (!term) return;
  try {
    const terms = await invoke("add_bundle_exclude_term", { term });
    input.value = "";
    renderBundleExcludeTerms(terms);
  } catch (e) {
    alert(String(e));
  }
}

document.getElementById("bundle-exclude-add-btn").addEventListener("click", addBundleExcludeTerm);
document.getElementById("bundle-exclude-input").addEventListener("keydown", (e) => {
  if (e.key === "Enter") addBundleExcludeTerm();
});

function setPill(id, isSet) {
  const el = document.getElementById(id);
  el.textContent = isSet ? "configured" : "not set";
  el.classList.toggle("set", isSet);
}

document.getElementById("save-settings-btn").addEventListener("click", async () => {
  const fields = [
    ["cred-humble", "humble_cookie"],
    ["cred-packt", "packt_cookies"],
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
// Loaded eagerly (not just on first Settings-tab visit) so
// `bundleExcludeTerms` is populated before the first "Check against
// current bundles" run, in case the user configured terms in a prior
// session and goes straight to the Check tab.
loadSettings();
