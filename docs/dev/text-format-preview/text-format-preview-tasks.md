---
feature: text-format-preview
version: 1
status: draft
parent: text-format-preview-design.md
---

# Task Breakdown: Text Format Preview

## Phase 4: Environment Preparation

- [ ] **T-001: Environment Setup**
    - **Goal**: Prepare the worktree and verify dependencies.
    - **Details**:
        - Create worktree `feature/text-format-preview`.
        - Verify `src/server.py` and `src/directory_handler.py` are writable.
        - Create `src/templates.py` (or similar) if we decide to separate HTML storage, or plan to modify `directory_handler.py`.
    - **Verification**: Branch exists, codebase is ready.

## Phase 5: Implementation

### Group 1: Backend API Foundation

- [ ] **T-002: API Routing Infrastructure**
    - **Goal**: Enable `DirectoryShareHandler` to distinguish between UI and API requests.
    - **Files**: `src/server.py`
    - **Steps**:
        - Modify `do_GET` to check if path starts with `/api/`.
        - Dispatch to `_handle_api_request` if it matches.
        - Implement basic 404/500 JSON error responses.
    - **Test**: `curl localhost:8000/api/test` returns JSON error instead of HTML.

- [ ] **T-003: Tree API Implementation**
    - **Goal**: Serve directory structure as JSON.
    - **Files**: `src/server.py`, `src/directory_handler.py`
    - **Steps**:
        - Implement `get_directory_structure(path)` helper.
        - Return JSON with name, type (file/dir), size, modified_time.
        - Ensure security validation works for API paths.
    - **Test**: `GET /api/tree?path=/` returns JSON list of root files.

- [ ] **T-004: Content API Implementation**
    - **Goal**: Serve file content with safeguards.
    - **Files**: `src/server.py`
    - **Steps**:
        - Implement `GET /api/content?path=...`.
        - Add check for file size (max 1MB for preview).
        - Add check for binary files (try/except decode utf-8).
        - Return JSON: `{ content: "...", encoding: "utf-8", type: "..." }`.
    - **Test**: `GET /api/content?path=README.md` returns file content.

### Group 2: Frontend Implementation

- [ ] **T-005: SPA Shell & Template**
    - **Goal**: Serve the Vue.js Single Page Application.
    - **Files**: `src/directory_handler.py`
    - **Steps**:
        - Create `generate_spa_html()` function.
        - Embed Vue 3, Prism.js, and Marked.js via CDN (or local fallback logic if needed, strictly CDN for MVP).
        - Setup basic layout: Sidebar (Tree) + Main (Content).
    - **Test**: accessing `/` (with new flag or toggled) loads the Vue app title.

- [ ] **T-006: Frontend Logic - File Tree**
    - **Goal**: Navigate directories in the browser.
    - **Files**: `src/directory_handler.py` (Javascript section)
    - **Steps**:
        - Implement `fetchTree(path)` in Vue.
        - Render recursive list of files/folders.
        - Handle folder expand/collapse.
    - **Test**: Can navigate folder structure without page reload.

- [ ] **T-007: Frontend Logic - File Preview**
    - **Goal**: Display file content with highlighting.
    - **Files**: `src/directory_handler.py` (Javascript section)
    - **Steps**:
        - Implement `fetchContent(path)` in Vue.
        - Detect file type by extension.
        - Apply `Prism.highlight` for code.
        - Apply `marked.parse` for Markdown.
    - **Test**: Clicking `server.py` shows colored code; `README.md` shows rendered HTML.

### Group 3: Integration & Polish

- [ ] **T-008: Default View Toggle & Clean up**
    - **Goal**: Decide how to switch between Legacy HTML and New SPA.
    - **Files**: `src/server.py`, `src/main.py`
    - **Steps**:
        - Add CLI flag `--spa` or `--preview` (or make it default if stable).
        - Ensure "Download Zip" still works from the UI.
        - Final code cleanup.
    - **Test**: End-to-end usage.
