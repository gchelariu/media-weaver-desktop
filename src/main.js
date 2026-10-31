const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const elements = {};
const results = [];
let isRunning = false;
let activeFilter = "all";

window.addEventListener("DOMContentLoaded", async () => {
  for (const id of [
    "directory", "browse-button", "dry-run", "run-button", "run-status",
    "scanned-count", "changed-count", "changed-label", "skipped-count", "unmatched-count",
    "errors-count", "file-count", "result-filter", "filter-trigger", "filter-label",
    "filter-menu", "empty-state",
    "results-table", "results-body",
  ]) {
    elements[id] = document.getElementById(id);
  }

  elements["browse-button"].addEventListener("click", chooseDirectory);
  elements["run-button"].addEventListener("click", runProcessor);
  elements["directory"].addEventListener("input", updateRunButton);
  elements["filter-trigger"].addEventListener("click", () => {
    setFilterMenuOpen(elements["filter-menu"].hidden);
  });
  elements["filter-trigger"].addEventListener("keydown", onFilterTriggerKeydown);
  elements["filter-menu"].addEventListener("click", onFilterOptionClick);
  elements["filter-menu"].addEventListener("keydown", onFilterMenuKeydown);
  document.addEventListener("pointerdown", (event) => {
    if (!elements["result-filter"].contains(event.target)) setFilterMenuOpen(false);
  });
  elements["dry-run"].addEventListener("change", () => {
    elements["changed-label"].textContent = elements["dry-run"].checked ? "Would change" : "Changed";
  });

  try {
    await listen("media-progress", ({ payload }) => {
      results.push(payload);
      renderResults();
      updateCounters();
    });
  } catch (error) {
    setStatus(`Could not receive scan updates: ${error}`, "error");
  }
});

async function chooseDirectory() {
  elements["browse-button"].disabled = true;
  try {
    const path = await invoke("choose_directory");
    if (path) {
      elements["directory"].value = path;
      updateRunButton();
      setStatus("Folder ready to scan.");
    }
  } catch (error) {
    setStatus(`Could not open the folder picker: ${error}`, "error");
  } finally {
    elements["browse-button"].disabled = false;
  }
}

async function runProcessor() {
  const root = elements["directory"].value.trim();
  if (!root || isRunning) return;

  isRunning = true;
  results.length = 0;
  renderResults();
  updateCounters();
  updateRunButton();
  elements["browse-button"].disabled = true;
  setStatus(elements["dry-run"].checked ? "Previewing files…" : "Updating metadata…", "working");

  try {
    const report = await invoke("process_media", {
      root,
      dryRun: elements["dry-run"].checked,
    });
    elements["scanned-count"].textContent = report.scanned;
    elements["changed-count"].textContent = report.changed;
    elements["skipped-count"].textContent = report.skipped;
    elements["unmatched-count"].textContent = report.outOfStructure;
    elements["errors-count"].textContent = report.errors;
    const structureMessage = report.outOfStructure
      ? ` ${report.outOfStructure} ${report.outOfStructure === 1 ? "file is" : "files are"} outside the YEAR/MM folder layout.`
      : "";
    setStatus(
      `Found ${report.scanned} media file${report.scanned === 1 ? "" : "s"}.${structureMessage}`,
      report.errors || report.outOfStructure ? "warning" : "success",
    );
  } catch (error) {
    setStatus(String(error), "error");
  } finally {
    isRunning = false;
    elements["browse-button"].disabled = false;
    updateRunButton();
  }
}

function updateRunButton() {
  elements["run-button"].disabled = isRunning || !elements["directory"].value.trim();
}

function updateCounters() {
  const counts = { scanned: 0, changed: 0, skipped: 0, unmatched: 0, errors: 0 };
  for (const result of results) {
    if (result.scanned) counts.scanned++;
    if (result.outcome === "changed") counts.changed++;
    else if (result.outcome === "skipped") counts.skipped++;
    else if (result.outcome === "unmatched") counts.unmatched++;
    else counts.errors++;
  }
  elements["scanned-count"].textContent = counts.scanned;
  elements["changed-count"].textContent = counts.changed;
  elements["skipped-count"].textContent = counts.skipped;
  elements["unmatched-count"].textContent = counts.unmatched;
  elements["errors-count"].textContent = counts.errors;
}

function renderResults() {
  const visible = results.filter((item) => activeFilter === "all" || item.outcome === activeFilter);
  elements["results-body"].replaceChildren();
  for (const result of [...visible].reverse()) {
    const row = document.createElement("tr");
    const file = document.createElement("td");
    const path = document.createElement("span");
    path.className = "file-path";
    path.textContent = result.path;
    path.title = result.path;
    file.append(path);

    const status = document.createElement("td");
    const badge = document.createElement("span");
    badge.className = `badge ${result.outcome}`;
    badge.textContent = result.outcome === "changed" && result.dryRun
      ? "Would change"
      : result.outcome === "unmatched"
        ? "Out of structure"
        : titleCase(result.outcome);
    status.append(badge);

    const details = document.createElement("td");
    details.className = result.outcome === "error" ? "details error-text" : "details";
    details.textContent = result.message;
    if (result.targetDate) details.title = `Target date: ${result.targetDate}`;

    row.append(file, status, details);
    elements["results-body"].append(row);
  }
  elements["empty-state"].hidden = results.length > 0;
  elements["results-table"].hidden = visible.length === 0;
  elements["file-count"].textContent = results.length
    ? `${visible.length} shown · ${results.length} total`
    : "No files yet";
}

function onFilterOptionClick(event) {
  const option = event.target.closest("[data-filter]");
  if (!option) return;

  activeFilter = option.dataset.filter;
  elements["filter-label"].textContent = option.textContent;
  for (const item of elements["filter-menu"].querySelectorAll("[data-filter]")) {
    item.setAttribute("aria-checked", String(item === option));
  }
  setFilterMenuOpen(false);
  elements["filter-trigger"].focus();
  renderResults();
}

function onFilterTriggerKeydown(event) {
  if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
  event.preventDefault();
  setFilterMenuOpen(true);
  const selected = elements["filter-menu"].querySelector('[aria-checked="true"]');
  selected?.focus();
}

function onFilterMenuKeydown(event) {
  const options = [...elements["filter-menu"].querySelectorAll("[data-filter]")];
  const currentIndex = options.indexOf(document.activeElement);
  let nextIndex;
  if (event.key === "Escape") {
    event.preventDefault();
    setFilterMenuOpen(false);
    elements["filter-trigger"].focus();
    return;
  } else if (event.key === "ArrowDown") {
    nextIndex = (currentIndex + 1) % options.length;
  } else if (event.key === "ArrowUp") {
    nextIndex = (currentIndex - 1 + options.length) % options.length;
  } else if (event.key === "Home") {
    nextIndex = 0;
  } else if (event.key === "End") {
    nextIndex = options.length - 1;
  } else {
    return;
  }
  event.preventDefault();
  options[nextIndex]?.focus();
}

function setFilterMenuOpen(open) {
  elements["filter-menu"].hidden = !open;
  elements["filter-trigger"].setAttribute("aria-expanded", String(open));
}

function setStatus(message, state = "") {
  elements["run-status"].textContent = message;
  elements["run-status"].className = `run-status ${state}`;
}

function titleCase(value) {
  return value.charAt(0).toUpperCase() + value.slice(1);
}
