"use strict";

const query = new URLSearchParams(window.location.search);
const token = query.get("token") || "";
const catalog = document.getElementById("catalog");
const status = document.getElementById("connection-status");
const downloadAll = document.getElementById("download-all");
const uploadPanel = document.getElementById("upload-panel");
const dropZone = document.getElementById("drop-zone");
const fileInput = document.getElementById("file-input");
const passwordField = document.getElementById("upload-password-field");
const passwordInput = document.getElementById("upload-password");
const uploadProgress = document.getElementById("upload-progress");

function formatBytes(value) {
  if (value < 1024) return `${value} B`;
  const units = ["KiB", "MiB", "GiB", "TiB"];
  let amount = value;
  let index = -1;
  do { amount /= 1024; index += 1; } while (amount >= 1024 && index < units.length - 1);
  return `${amount.toFixed(amount >= 10 ? 1 : 2)} ${units[index]}`;
}

function setStatus(label, mode) {
  status.textContent = label;
  status.className = `status status--${mode}`;
}

function isPreviewable(mediaType) {
  return [
    "text/plain", "text/csv", "application/json", "image/png", "image/jpeg",
    "image/gif", "image/webp", "image/bmp"
  ].includes(mediaType);
}

function makeEntry(entry, childrenByParent) {
  const wrapper = document.createElement("div");
  wrapper.className = "catalog-entry";
  const row = document.createElement("div");
  row.className = "catalog-row";

  const name = document.createElement("span");
  name.className = "catalog-row__name";
  name.textContent = entry.kind === "directory" ? `Folder · ${entry.name}` : entry.name;
  row.appendChild(name);

  const metadata = document.createElement("span");
  metadata.className = "catalog-row__meta";
  metadata.textContent = entry.kind === "directory" ? "Directory" : formatBytes(entry.size);
  row.appendChild(metadata);

  if (entry.kind === "file") {
    const actions = document.createElement("span");
    actions.className = "catalog-row__actions";
    if (isPreviewable(entry.mediaType)) {
      const preview = document.createElement("a");
      preview.className = "catalog-row__action";
      preview.textContent = "Preview";
      preview.target = "_blank";
      preview.rel = "noopener";
      preview.href = `/api/preview/${encodeURIComponent(entry.id)}?token=${encodeURIComponent(token)}`;
      actions.appendChild(preview);
    }
    const link = document.createElement("a");
    link.className = "catalog-row__action";
    link.textContent = "Download";
    link.href = `/api/download/${encodeURIComponent(entry.id)}?token=${encodeURIComponent(token)}`;
    actions.appendChild(link);
    row.appendChild(actions);
  }
  wrapper.appendChild(row);

  if (entry.kind === "directory") {
    const children = document.createElement("div");
    children.className = "catalog-children";
    (childrenByParent.get(entry.id) || []).forEach((child) => children.appendChild(makeEntry(child, childrenByParent)));
    const toggle = document.createElement("button");
    toggle.type = "button";
    toggle.className = "catalog-row__action";
    toggle.textContent = "Collapse";
    toggle.addEventListener("click", () => {
      children.hidden = !children.hidden;
      toggle.textContent = children.hidden ? "Expand" : "Collapse";
    });
    row.appendChild(toggle);
    wrapper.appendChild(children);
  }
  return wrapper;
}

function renderCatalog(entries) {
  const childrenByParent = new Map();
  entries.forEach((entry) => {
    const key = entry.parentId || "root";
    if (!childrenByParent.has(key)) childrenByParent.set(key, []);
    childrenByParent.get(key).push(entry);
  });
  (childrenByParent.get("root") || []).forEach((entry) => catalog.appendChild(makeEntry(entry, childrenByParent)));
}

async function loadCatalog() {
  if (!token) {
    setStatus("Access denied", "error");
    catalog.textContent = "This link is missing its private access token.";
    return;
  }
  try {
    const response = await fetch(`/api/catalog?token=${encodeURIComponent(token)}`, { cache: "no-store" });
    if (!response.ok) throw new Error(`Catalog request failed (${response.status})`);
    const payload = await response.json();
    uploadPanel.hidden = !payload.uploadEnabled;
    passwordField.hidden = !payload.uploadPasswordRequired;
    passwordInput.required = payload.uploadPasswordRequired;
    catalog.textContent = "";
    if (payload.entries.length === 0) {
      catalog.textContent = "No files are currently shared.";
    } else {
      renderCatalog(payload.entries);
    }
    downloadAll.href = `/api/download-all?token=${encodeURIComponent(token)}`;
    setStatus("Ready", "ready");
  } catch (error) {
    setStatus("Unavailable", "error");
    catalog.textContent = error instanceof Error ? error.message : "Catalog unavailable";
  }
}

function renderUploadProgress(label, percent) {
  uploadProgress.textContent = "";
  const text = document.createElement("span");
  text.textContent = label;
  const progress = document.createElement("progress");
  progress.className = "progress-track";
  progress.max = 100;
  progress.value = Math.max(0, Math.min(100, percent));
  uploadProgress.appendChild(text);
  uploadProgress.appendChild(progress);
}

function uploadFiles(files) {
  if (!files || files.length === 0) return;
  const form = new FormData();
  Array.from(files).forEach((file) => form.append("files", file, file.name));
  const request = new XMLHttpRequest();
  request.open("POST", `/api/upload?token=${encodeURIComponent(token)}`);
  if (passwordInput.value) request.setRequestHeader("X-Quick-Share-Upload-Password", passwordInput.value);
  request.upload.addEventListener("progress", (event) => {
    const percent = event.lengthComputable ? (event.loaded / event.total) * 100 : 0;
    renderUploadProgress(`Uploading ${files.length} file(s)…`, percent);
  });
  request.addEventListener("load", () => {
    if (request.status === 201) {
      renderUploadProgress("Upload complete", 100);
    } else {
      renderUploadProgress(`Upload failed (${request.status})`, 0);
    }
  });
  request.addEventListener("error", () => renderUploadProgress("Upload connection failed", 0));
  request.send(form);
}

["dragenter", "dragover"].forEach((name) => dropZone.addEventListener(name, (event) => {
  event.preventDefault();
  dropZone.classList.add("drop-zone--active");
}));
["dragleave", "drop"].forEach((name) => dropZone.addEventListener(name, (event) => {
  event.preventDefault();
  dropZone.classList.remove("drop-zone--active");
}));
dropZone.addEventListener("drop", (event) => uploadFiles(event.dataTransfer.files));
fileInput.addEventListener("change", () => uploadFiles(fileInput.files));

loadCatalog();
