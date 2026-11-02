const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const elements = {};
const results = [];
let isRunning = false;
let isDuplicateScanRunning = false;
let duplicateCardsObserver = null;
let activeFilter = "all";

window.addEventListener("DOMContentLoaded", async () => {
  for (const id of [
    "directory", "browse-button", "dry-run", "run-button", "run-status",
    "scanned-count", "changed-count", "changed-label", "skipped-count", "unmatched-count",
    "errors-count", "file-count", "result-filter", "filter-trigger", "filter-label",
    "filter-menu", "empty-state", "metadata-groups",
    "results-table", "results-body",
    "processor-tab", "duplicates-tab", "processor-panel", "duplicates-panel",
    "duplicates-directory", "duplicates-browse-button", "duplicates-run-button",
    "duplicates-status", "duplicates-scanned-count", "duplicate-groups-count",
    "duplicate-files-count", "duplicate-errors-count", "duplicate-file-count", "duplicates-empty-state",
    "duplicate-groups", "duplicate-error-details", "delete-confirmation",
    "delete-confirmation-details",
  ]) {
    elements[id] = document.getElementById(id);
  }

  loadMetadataFields();
  elements["browse-button"].addEventListener("click", chooseDirectory);
  elements["duplicates-browse-button"].addEventListener("click", chooseDuplicatesDirectory);
  elements["run-button"].addEventListener("click", runProcessor);
  elements["duplicates-run-button"].addEventListener("click", runDuplicateScan);
  elements["duplicate-groups"].addEventListener("click", onDuplicateDeleteClick);
  elements["processor-tab"].addEventListener("click", () => setActiveTab("processor"));
  elements["duplicates-tab"].addEventListener("click", () => setActiveTab("duplicates"));
  for (const tab of [elements["processor-tab"], elements["duplicates-tab"]]) {
    tab.addEventListener("keydown", onTabKeydown);
  }
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

async function loadMetadataFields() {
  try {
    const fields = await invoke("get_metadata_fields");
    renderMetadataFields(fields);
  } catch (error) {
    const message = document.createElement("p");
    message.className = "metadata-loading error-text";
    message.textContent = `Could not load configured metadata fields: ${error}`;
    elements["metadata-groups"].replaceChildren(message);
  }
}

function renderMetadataFields(fields) {
  const groups = [
    { key: "images", title: "Images MetaDates", className: "" },
    { key: "videos", title: "Videos MetaDates", className: "video-metadata" },
  ];
  const groupElements = groups.map(({ key, title, className }) => {
    const article = document.createElement("article");
    article.className = `metadata-group ${className}`.trim();

    const heading = document.createElement("h3");
    heading.textContent = title;

    const list = document.createElement("ul");
    list.className = "metadata-tags";
    for (const field of fields[key] ?? []) {
      const item = document.createElement("li");
      item.append(document.createTextNode(field));
      if (field === "FileModifyDate") {
        const note = document.createElement("span");
        note.textContent = " (file system)";
        item.append(note);
      }
      list.append(item);
    }

    article.append(heading, list);
    return article;
  });
  elements["metadata-groups"].replaceChildren(...groupElements);
}

async function chooseDirectory() {
  await chooseFolder(elements["browse-button"], elements["directory"], setStatus);
}

async function chooseDuplicatesDirectory() {
  await chooseFolder(elements["duplicates-browse-button"], elements["duplicates-directory"], setDuplicateStatus);
}

async function chooseFolder(button, input, status) {
  button.disabled = true;
  try {
    const path = await invoke("choose_directory");
    if (path) {
      const changedFolder = input.value !== path;
      input.value = path;
      if (input === elements["directory"]) updateRunButton();
      updateDuplicateRunButton();
      if (input === elements["duplicates-directory"] && changedFolder) clearDuplicateResults();
      status("Folder ready to scan.");
    }
  } catch (error) {
    status(`Could not open the folder picker: ${error}`, "error");
  } finally {
    button.disabled = false;
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

async function runDuplicateScan() {
  const root = elements["duplicates-directory"].value.trim();
  if (!root || isDuplicateScanRunning) return;

  isDuplicateScanRunning = true;
  clearDuplicateResults();
  elements["duplicates-empty-state"].hidden = false;
  elements["duplicates-empty-state"].querySelector("strong").textContent = "Scanning folder structure…";
  elements["duplicates-empty-state"].querySelector("span:last-child").textContent = "Looking through all nested folders.";
  elements["duplicate-file-count"].textContent = "Scanning…";
  updateDuplicateRunButton();
  elements["duplicates-browse-button"].disabled = true;
  setDuplicateStatus("Scanning every folder…", "working");

  try {
    const report = await invoke("scan_duplicate_names", { root });
    renderDuplicateResults(report);
    setDuplicateStatus(
      `Scanned ${report.scannedFiles} file${report.scannedFiles === 1 ? "" : "s"}; found ${report.duplicateNames} duplicate name${report.duplicateNames === 1 ? "" : "s"} across ${report.directories.length} director${report.directories.length === 1 ? "y" : "ies"}.`,
      report.errors.length ? "warning" : "success",
    );
  } catch (error) {
    setDuplicateStatus(String(error), "error");
    elements["duplicates-empty-state"].querySelector("strong").textContent = "Could not scan this folder";
    elements["duplicates-empty-state"].querySelector("span:last-child").textContent = "Check that the selected folder is still available, then try again.";
  } finally {
    isDuplicateScanRunning = false;
    elements["duplicates-browse-button"].disabled = false;
    updateDuplicateRunButton();
  }
}

function clearDuplicateResults() {
  duplicateCardsObserver?.disconnect();
  duplicateCardsObserver = null;
  elements["duplicate-groups"].replaceChildren();
  elements["duplicate-groups"].hidden = true;
  elements["duplicate-error-details"].textContent = "";
  elements["duplicate-error-details"].hidden = true;
  elements["duplicates-empty-state"].hidden = false;
  elements["duplicates-empty-state"].querySelector("strong").textContent = "No duplicate scan yet";
  elements["duplicates-empty-state"].querySelector("span:last-child").textContent = "Choose a folder and scan to find repeated file names.";
  elements["duplicates-scanned-count"].textContent = "0";
  elements["duplicate-groups-count"].textContent = "0";
  elements["duplicate-files-count"].textContent = "0";
  elements["duplicate-errors-count"].textContent = "0";
  elements["duplicate-file-count"].textContent = "No files yet";
}

function renderDuplicateResults(report) {
  const directories = report.directories;
  elements["duplicates-scanned-count"].textContent = report.scannedFiles;
  elements["duplicate-groups-count"].textContent = report.duplicateNames;
  elements["duplicate-files-count"].textContent = report.duplicateFiles;
  elements["duplicate-errors-count"].textContent = report.errors.length;
  elements["duplicate-file-count"].textContent = `${report.duplicateFiles} matching file${report.duplicateFiles === 1 ? "" : "s"} · ${directories.length} director${directories.length === 1 ? "y" : "ies"}`;

  const groupElements = directories.map((group) => {
    const article = document.createElement("article");
    article.className = "duplicate-group";
    const matchingFileCount = group.duplicateDirectories.reduce(
      (count, duplicateDirectory) => count + duplicateDirectory.fileNames.length,
      0,
    );

    const heading = document.createElement("div");
    heading.className = "duplicate-group-heading";
    const directory = document.createElement("h4");
    directory.textContent = group.directory;
    const count = document.createElement("span");
    count.className = "duplicate-count";
    count.textContent = `${matchingFileCount} matching file${matchingFileCount === 1 ? "" : "s"}`;
    heading.append(directory, count);

    const counterparts = document.createElement("ul");
    counterparts.className = "duplicate-counterparts";
    for (const duplicateDirectory of group.duplicateDirectories) {
      const counterpart = document.createElement("li");
      counterpart.className = "duplicate-counterpart";

      const counterpartHeading = document.createElement("div");
      counterpartHeading.className = "duplicate-counterpart-heading";
      const counterpartPath = document.createElement("h5");
      counterpartPath.textContent = duplicateDirectory.directory;
      counterpartPath.title = duplicateDirectory.directory;
      const counterpartCount = document.createElement("span");
      counterpartCount.textContent = `${duplicateDirectory.fileNames.length} file${duplicateDirectory.fileNames.length === 1 ? "" : "s"}`;
      counterpartHeading.append(counterpartPath, counterpartCount);

      const fileNames = document.createElement("ul");
      fileNames.className = "duplicate-file-names";
      for (const name of duplicateDirectory.fileNames) {
        const fileName = document.createElement("li");
        fileName.className = "duplicate-file-name";
        fileName.textContent = name;
        fileName.title = name;
        fileNames.append(fileName);
      }

      const deleteAction = document.createElement("div");
      deleteAction.className = "duplicate-delete-action";
      const deleteScope = document.createElement("p");
      deleteScope.textContent = "Only the files listed above in this directory will be deleted.";

      const deleteButton = document.createElement("button");
      deleteButton.className = "button danger delete-duplicates-button";
      deleteButton.type = "button";
      deleteButton.dataset.deleteDuplicates = "true";
      deleteButton.dataset.directory = duplicateDirectory.directory;
      deleteButton.dataset.fileNames = JSON.stringify(duplicateDirectory.fileNames);
      deleteButton.textContent = `Delete these ${duplicateDirectory.fileNames.length} files`;
      deleteButton.setAttribute(
        "aria-label",
        `Delete ${duplicateDirectory.fileNames.length} duplicate files from ${duplicateDirectory.directory}`,
      );
      deleteAction.append(deleteScope, deleteButton);
      counterpart.append(counterpartHeading, fileNames, deleteAction);
      counterparts.append(counterpart);
    }
    article.append(heading, counterparts);
    return article;
  });
  elements["duplicate-groups"].replaceChildren(...groupElements);
  animateDuplicateCards();
  elements["duplicate-groups"].hidden = directories.length === 0;
  elements["duplicates-empty-state"].hidden = directories.length > 0;
  if (directories.length === 0) {
    elements["duplicates-empty-state"].querySelector("strong").textContent = "No duplicate file names found";
    elements["duplicates-empty-state"].querySelector("span:last-child").textContent = `Scanned ${report.scannedFiles} file${report.scannedFiles === 1 ? "" : "s"} across the selected folder.`;
  }

  if (report.errors.length) {
    const more = report.errors.length > 3 ? ` (and ${report.errors.length - 3} more)` : "";
    elements["duplicate-error-details"].textContent = `Some folders could not be scanned: ${report.errors.slice(0, 3).join(" · ")}${more}`;
    elements["duplicate-error-details"].hidden = false;
  }
}

function animateDuplicateCards() {
  duplicateCardsObserver?.disconnect();
  duplicateCardsObserver = null;
  const cards = elements["duplicate-groups"].querySelectorAll(".duplicate-group, .duplicate-counterpart");
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches || !("IntersectionObserver" in window)) {
    for (const card of cards) card.classList.add("fade-in");
    return;
  }

  const observer = new IntersectionObserver((entries) => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      entry.target.classList.add("fade-in");
      observer.unobserve(entry.target);
    }
  });
  duplicateCardsObserver = observer;

  for (const card of cards) observer.observe(card);
}

async function onDuplicateDeleteClick(event) {
  const button = event.target.closest("[data-delete-duplicates]");
  if (!button || isDuplicateScanRunning) return;

  let fileNames;
  try {
    fileNames = JSON.parse(button.dataset.fileNames);
  } catch {
    setDuplicateStatus("Could not read the selected duplicate files.", "error");
    return;
  }

  const directory = button.dataset.directory;
  const root = elements["duplicates-directory"].value.trim();
  if (!root) return;
  const confirmed = await confirmDuplicateDeletion(directory, fileNames.length);
  if (!confirmed) return;

  isDuplicateScanRunning = true;
  elements["duplicates-browse-button"].disabled = true;
  updateDuplicateRunButton();
  for (const deleteButton of elements["duplicate-groups"].querySelectorAll("[data-delete-duplicates]")) {
    deleteButton.disabled = true;
  }
  setDuplicateStatus(`Deleting duplicates from ${directory}…`, "working");

  try {
    const deletion = await invoke("delete_duplicate_files", {
      root,
      directory,
      fileNames,
    });
    const report = await invoke("scan_duplicate_names", { root });
    renderDuplicateResults(report);

    const warningCount = deletion.errors.length + report.errors.length;
    const warning = deletion.errors.length
      ? ` ${deletion.errors.length} file${deletion.errors.length === 1 ? " could" : "s could"} not be deleted.`
      : report.errors.length
        ? " Some folders could not be rescanned."
        : "";
    setDuplicateStatus(
      `Deleted ${deletion.deletedFiles} file${deletion.deletedFiles === 1 ? "" : "s"} from ${directory}.${warning}`,
      warningCount ? "warning" : "success",
    );
  } catch (error) {
    setDuplicateStatus(`Could not complete deletion: ${error}`, "error");
    try {
      renderDuplicateResults(await invoke("scan_duplicate_names", { root }));
    } catch {
      // Keep the deletion error visible if the follow-up scan also fails.
    }
  } finally {
    isDuplicateScanRunning = false;
    elements["duplicates-browse-button"].disabled = false;
    updateDuplicateRunButton();
    for (const deleteButton of elements["duplicate-groups"].querySelectorAll("[data-delete-duplicates]")) {
      deleteButton.disabled = false;
    }
  }
}

function confirmDuplicateDeletion(directory, fileCount) {
  elements["delete-confirmation-details"].textContent =
    `Delete ${fileCount} duplicate file${fileCount === 1 ? "" : "s"} from ${directory}?`;
  const dialog = elements["delete-confirmation"];
  dialog.returnValue = "";
  return new Promise((resolve) => {
    dialog.addEventListener("close", () => resolve(dialog.returnValue === "continue"), { once: true });
    dialog.showModal();
  });
}

function updateDuplicateRunButton() {
  elements["duplicates-run-button"].disabled = isDuplicateScanRunning || !elements["duplicates-directory"].value.trim();
}

function setActiveTab(tabName) {
  const isProcessor = tabName === "processor";
  elements["processor-tab"].classList.toggle("active", isProcessor);
  elements["processor-tab"].setAttribute("aria-selected", String(isProcessor));
  elements["processor-tab"].tabIndex = isProcessor ? 0 : -1;
  elements["duplicates-tab"].classList.toggle("active", !isProcessor);
  elements["duplicates-tab"].setAttribute("aria-selected", String(!isProcessor));
  elements["duplicates-tab"].tabIndex = isProcessor ? -1 : 0;
  elements["processor-panel"].hidden = !isProcessor;
  elements["duplicates-panel"].hidden = isProcessor;
}

function onTabKeydown(event) {
  if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
  event.preventDefault();
  const nextTab = event.currentTarget === elements["processor-tab"] ? "duplicates" : "processor";
  setActiveTab(nextTab);
  elements[`${nextTab}-tab`].focus();
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

function setDuplicateStatus(message, state = "") {
  elements["duplicates-status"].textContent = message;
  elements["duplicates-status"].className = `run-status ${state}`;
}

function titleCase(value) {
  return value.charAt(0).toUpperCase() + value.slice(1);
}
