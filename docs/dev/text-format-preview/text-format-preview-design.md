---
feature: text-format-preview
version: 1
status: draft
---

# Technical Design: Text Format Preview

## 1. Architecture Overview

The "Text Format Preview" feature transforms the current server-side rendered directory listing into a client-side Single Page Application (SPA) powered by Vue.js. This decoupling allows for a richer user experience with instant file previews, syntax highlighting, and markdown rendering.

### 1.1 Components

1.  **Backend (Python)**:
    -   Extends `DirectoryShareHandler` to support JSON APIs.
    -   Serves the static SPA entry point.
    -   Provides security validation for API requests.

2.  **Frontend (Vue.js)**:
    -   **Framework**: Vue.js 3 (ES Module build via CDN/Local).
    -   **UI Library**: Custom CSS (minimal dependencies).
    -   **Syntax Highlight**: Prism.js (lightweight).
    -   **Markdown**: Marked.js.
    -   **State Management**: Vue Reactivity API.

## 2. API Design

### 2.1 GET /api/tree
Returns the directory structure for a given path.

**Request:**
- `path`: Relative path from the root shared directory (default: `/`).

**Response (200 OK):**
```json
{
  "path": "/src",
  "items": [
    {
      "name": "main.py",
      "type": "file",
      "size": 1024,
      "modified": "2023-01-01T12:00:00Z",
      "extension": ".py"
    },
    {
      "name": "utils",
      "type": "directory",
      "modified": "2023-01-01T12:00:00Z"
    }
  ]
}
```

### 2.2 GET /api/content
Returns the raw content of a text file.

**Request:**
- `path`: Relative path to the file.

**Response (200 OK):**
```json
{
  "path": "/src/main.py",
  "content": "import os...",
  "size": 1024,
  "encoding": "utf-8",
  "type": "text/x-python"
}
```

**Errors:**
- `400 Bad Request`: Path traversal attempt.
- `403 Forbidden`: Access denied (outside root).
- `404 Not Found`: File doesn't exist.
- `413 Payload Too Large`: File exceeds preview limit (e.g., 1MB).

## 3. Module Design

### 3.1 Server Modifications (`src/server.py`)
-   Modify `DirectoryShareHandler.do_GET` to intercept `/api/` requests.
-   Implement `handle_api_tree` and `handle_api_content`.
-   Add `handle_spa_serve` to serve the Vue.js application HTML.

### 3.2 Directory Handler (`src/directory_handler.py`)
-   Add `generate_spa_html()`: Returns the shell HTML for the Vue app.
-   This HTML will include CDN links to Vue, Prism, and Marked (with fallbacks if possible, or just CDN for MVP).

### 3.3 Security (`src/security.py`)
-   Reuse existing `validate_directory_path` for API requests.
-   Ensure API responses don't leak absolute server paths.

## 4. Frontend Architecture

### 4.1 State Management (Store)
```javascript
const state = reactive({
  currentPath: '/',
  tree: [],
  selectedFile: null,
  fileContent: '',
  isLoading: false,
  error: null
});
```

### 4.2 Components
1.  **FileTree**: Recursive component or flattened list to navigate directories.
2.  **FilePreview**:
    -   **CodeView**: Uses Prism.js for syntax highlighting.
    -   **MarkdownView**: Uses Marked.js for rendering.
    -   **RawView**: Fallback for plain text.

## 5. Implementation Strategy

### 5.1 Dependency Management
Since this is a Python package distributed via pip, we want to avoid a complex Node.js build step during installation.
-   **Solution**: Embed the Frontend code (HTML/CSS/JS) as string templates in Python or serve static files from a `static/` directory included in `package_data`.
-   **Decision**: For MVP, we will inline the HTML/JS into `src/directory_handler.py` (similar to current HTML generation) but structured cleanly.

### 5.2 Security Constraints
-   **ReadOnly**: The API is strictly read-only.
-   **Sandboxing**: All paths must be validated against `base_dir`.

## 6. Detailed Tasks
(Will be expanded in Phase 3)
1.  Setup API skeletal routing in `server.py`.
2.  Implement `get_tree` logic in Python.
3.  Implement `get_content` logic with size limits.
4.  Create the Single Page Application HTML/JS template.
5.  Integrate and Test.
