import html
import os

def generate_spa_html(base_dir_name: str) -> str:
    """
    Generate the Single Page Application HTML.

    Args:
        base_dir_name: Name of the root directory being shared

    Returns:
        String containing the full HTML/JS/CSS for the SPA
    """
    return f"""<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Quick Share - {html.escape(base_dir_name)}</title>

    <!-- Dependencies -->
    <script src="https://unpkg.com/vue@3/dist/vue.global.js"></script>
    <script src="https://unpkg.com/prismjs@1.29.0/components/prism-core.min.js"></script>
    <script src="https://unpkg.com/prismjs@1.29.0/plugins/autoloader/prism-autoloader.min.js"></script>
    <script src="https://unpkg.com/marked@4.3.0/marked.min.js"></script>

    <!-- Styles -->
    <link rel="stylesheet" href="https://unpkg.com/prismjs@1.29.0/themes/prism-tomorrow.min.css">
    <style>
        :root {{
            --sidebar-width: 300px;
            --header-height: 60px;
            --bg-color: #f5f5f5;
            --sidebar-bg: #fff;
            --border-color: #ddd;
            --primary-color: #007bff;
            --text-color: #333;
        }}

        * {{ box-sizing: border-box; }}

        body {{
            margin: 0;
            font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
            background: var(--bg-color);
            color: var(--text-color);
            height: 100vh;
            display: flex;
            flex-direction: column;
        }}

        /* Header */
        header {{
            height: var(--header-height);
            background: #fff;
            border-bottom: 1px solid var(--border-color);
            display: flex;
            align-items: center;
            padding: 0 20px;
            justify-content: space-between;
        }}

        .brand {{
            font-weight: bold;
            font-size: 1.2rem;
            display: flex;
            align-items: center;
            gap: 10px;
        }}

        .actions {{
            display: flex;
            gap: 10px;
        }}

        .btn {{
            padding: 8px 16px;
            border-radius: 4px;
            text-decoration: none;
            font-size: 0.9rem;
            cursor: pointer;
            border: 1px solid var(--border-color);
            background: #f8f9fa;
            color: var(--text-color);
        }}

        .btn-primary {{
            background: var(--primary-color);
            color: white;
            border-color: var(--primary-color);
        }}

        /* Main Layout */
        .main-container {{
            flex: 1;
            display: flex;
            overflow: hidden;
        }}

        /* Sidebar */
        .sidebar {{
            width: var(--sidebar-width);
            background: var(--sidebar-bg);
            border-right: 1px solid var(--border-color);
            display: flex;
            flex-direction: column;
            overflow: hidden;
        }}

        .sidebar-header {{
            padding: 10px;
            border-bottom: 1px solid var(--border-color);
            font-weight: bold;
            color: #666;
            font-size: 0.8rem;
            flex-shrink: 0;
        }}

        .file-tree {{
            padding: 10px;
            flex: 1;
            overflow-y: auto;
        }}

        .tree-item {{
            cursor: pointer;
            padding: 4px 8px;
            border-radius: 4px;
            white-space: nowrap;
            overflow: hidden;
            text-overflow: ellipsis;
            display: flex;
            align-items: center;
            gap: 6px;
        }}

        .tree-item:hover {{
            background-color: #f0f0f0;
        }}

        .tree-item.active {{
            background-color: #e6f3ff;
            color: var(--primary-color);
        }}

        .tree-indent {{
            margin-left: 20px;
        }}

        /* Content Area */
        .content-area {{
            flex: 1;
            overflow-y: auto;
            padding: 20px;
            background: white;
            position: relative;
        }}

        .empty-state {{
            display: flex;
            flex-direction: column;
            align-items: center;
            justify-content: center;
            height: 100%;
            color: #888;
        }}

        .preview-container {{
            max-width: 1000px;
            margin: 0 auto;
        }}

        .markdown-body {{
            line-height: 1.6;
        }}

        pre {{
            border-radius: 6px;
            margin: 0;
        }}

        .loading-overlay {{
            position: absolute;
            top: 0;
            left: 0;
            right: 0;
            bottom: 0;
            background: rgba(255,255,255,0.8);
            display: flex;
            align-items: center;
            justify-content: center;
            z-index: 10;
        }}

        .error-message {{
            color: #dc3545;
            padding: 15px;
            background: #f8d7da;
            border-radius: 4px;
            margin-bottom: 20px;
        }}
        .upload-panel {{
            border-top: 1px solid var(--border-color);
            padding: 20px 0; margin-top: 20px;
        }}
        .upload-dropzone {{
            border: 2px dashed #ccc; border-radius: 6px; padding: 24px;
            text-align: center; cursor: pointer; transition: border-color 0.2s;
            margin: 8px 0;
        }}
        .upload-dropzone:hover, .upload-dropzone.dragover {{ border-color: var(--primary-color); background: #f0f7ff; }}
        .upload-progress {{ width: 100%; margin: 8px 0; }}
        .upload-status {{ font-size: 0.85rem; margin-top: 4px; }}
        .upload-status.success {{ color: #28a745; }}
        .upload-status.error {{ color: #dc3545; }}
    </style>
</head>
<body>
    <div id="app">
        <header>
            <div class="brand">
                <span>⚡ Quick Share</span>
                <span style="color: #888; font-weight: normal;">/ {html.escape(base_dir_name)}</span>
            </div>
            <div class="actions">
                <button class="btn" @click="toggleUpload">{{{{ showUpload ? 'Close Upload' : 'Upload' }}}}</button>
                <a href="/?legacy=1" class="btn">Legacy View</a>
                <a href="/?download=zip" class="btn">Download ZIP</a>
            </div>
        </header>

        <div class="main-container">
            <aside class="sidebar">
                <div class="sidebar-header">FILES</div>
                <div class="file-tree">
                    <tree-item
                        v-for="item in tree"
                        :key="item.name"
                        :item="item"
                        :current-path="currentPath"
                        @select="selectItem"
                    ></tree-item>
                </div>
            </aside>

            <main class="content-area">
                <div v-if="loading" class="loading-overlay">Loading...</div>

                <div v-if="error" class="error-message">
                    {{{{ error }}}}
                </div>

                <div v-if="selectedFile" class="preview-container">
                    <div class="file-info" style="margin-bottom: 15px; padding-bottom: 10px; border-bottom: 1px solid #eee;">
                        <h2 style="margin: 0;">{{{{ selectedFile.name }}}}</h2>
                        <div style="color: #666; font-size: 0.9rem; margin-top: 5px;">
                            {{{{ formatSize(selectedFile.size) }}}} • {{{{ selectedFile.modified }}}}
                        </div>
                    </div>

                    <div v-if="fileContent" class="file-content">
                        <!-- Markdown -->
                        <div v-if="isMarkdown" v-html="renderedMarkdown" class="markdown-body"></div>

                        <!-- Code -->
                        <pre v-else><code :class="languageClass">{{{{ fileContent }}}}</code></pre>
                    </div>
                </div>

                <div v-else class="empty-state">
                    <div style="font-size: 4rem; margin-bottom: 20px;">📄</div>
                    <h3>Select a file to preview</h3>
                    <p>Supported formats: Text, Code, Markdown</p>
                </div>

                <!-- Upload Section -->
                <div v-if="showUpload" class="upload-panel">
                    <h3 style="margin-bottom: 12px;">Upload Files</h3>
                    <div v-if="uploadPasswordRequired" style="margin-bottom:12px;">
                        <label style="display:block;margin-bottom:4px;font-size:0.85rem;color:#555;">Upload Password</label>
                        <input type="password" v-model="uploadPassword" placeholder="Enter upload password"
                               style="width:100%;max-width:300px;padding:10px 12px;border:1px solid #ccc;border-radius:6px;font-size:0.95rem;" />
                    </div>
                    <div :class="isDragging ? 'upload-dropzone dragover' : 'upload-dropzone'"
                         @click="triggerFileInput"
                         @dragover.prevent="onDragOver" @dragleave.prevent="onDragLeave"
                         @drop.prevent="onDrop">
                        <p v-if="!uploadSelectedFile">Drag &amp; drop a file here or click to select</p>
                        <p v-else>{{{{ uploadSelectedFile.name }}}} ({{{{ formatSize(uploadSelectedFile.size) }}}})</p>
                    </div>
                    <input type="file" ref="uploadFileInput" @change="onUploadFileSelected" style="display:none" />
                    <progress class="upload-progress" :value="uploadProgress" max="100" v-if="uploading"></progress>
                    <div :class="['upload-status', uploadStatusClass]" v-if="uploadMessage">{{{{ uploadMessage }}}}</div>
                    <button class="btn btn-primary" @click="startUpload" :disabled="!uploadSelectedFile || uploading"
                            style="margin-top: 8px;">
                        {{{{ uploading ? 'Uploading...' : 'Upload' }}}}
                    </button>
                </div>
            </main>
        </div>
    </div>

    <script>
        const {{ createApp, ref, computed, onMounted, watch }} = Vue;

        // Tree Item Component
        const TreeItem = {{
            name: 'TreeItem',
            props: ['item', 'currentPath'],
            emits: ['select'],
            setup(props, {{ emit }}) {{
                const isOpen = ref(false);
                const children = ref([]);
                const isLoading = ref(false);

                const isFolder = computed(() => props.item.type === 'directory');

                // Path is passed pre-calculated from parent
                const fullPath = computed(() => props.item.path);

                function toggle() {{
                    if (isFolder.value) {{
                        isOpen.value = !isOpen.value;
                        if (isOpen.value && children.value.length === 0) {{
                            loadChildren();
                        }}
                    }} else {{
                        emit('select', props.item);
                    }}
                }}

                async function loadChildren() {{
                    isLoading.value = true;
                    try {{
                        const res = await fetch(`/api/tree?path=${{encodeURIComponent(fullPath.value)}}`);
                        if (!res.ok) throw new Error('Failed to load');
                        const data = await res.json();

                        // Enrich children with paths
                        const parentPath = fullPath.value === '/' ? '' : fullPath.value;
                        children.value = data.items.map(child => ({{
                            ...child,
                            path: parentPath + '/' + child.name
                        }}));
                    }} catch (e) {{
                        console.error(e);
                    }} finally {{
                        isLoading.value = false;
                    }}
                }}

                return {{
                    isOpen,
                    isFolder,
                    children,
                    isLoading,
                    toggle
                }};
            }},
            template: `
                <div class="tree-node">
                    <div class="tree-item" @click="toggle" :class="{{ active: false }}">
                        <span style="width: 20px; text-align: center;">
                            {{{{ isFolder ? (isOpen ? '📂' : '📁') : '📄' }}}}
                        </span>
                        {{{{ item.name }}}}
                    </div>
                    <div v-if="isFolder && isOpen" class="tree-indent">
                         <div v-if="isLoading" style="color: #999; font-size: 0.8em; padding-left: 28px;">Loading...</div>
                         <tree-item
                            v-else
                            v-for="child in children"
                            :key="child.name"
                            :item="child"
                            :current-path="currentPath"
                            @select="$emit('select', $event)"
                         ></tree-item>
                         <div v-if="!isLoading && children.length === 0" style="color: #999; font-size: 0.8em; padding-left: 28px;">
                            (Empty)
                         </div>
                    </div>
                </div>
            `
        }};

        createApp({{
            components: {{ TreeItem }},
            setup() {{
                const tree = ref([]);
                const selectedFile = ref(null);
                const fileContent = ref('');
                const loading = ref(false);
                const error = ref(null);
                const currentPath = ref('/');

                // Computed
                const isMarkdown = computed(() => {{
                    return selectedFile.value && selectedFile.value.name.toLowerCase().endsWith('.md');
                }});

                const languageClass = computed(() => {{
                    if (!selectedFile.value) return '';
                    const ext = selectedFile.value.name.split('.').pop().toLowerCase();
                    return `language-${{ext}}`;
                }});

                const renderedMarkdown = computed(() => {{
                    if (!fileContent.value) return '';
                    return marked.parse(fileContent.value);
                }});

                // Methods
                function formatSize(bytes) {{
                    if (bytes === 0) return '0 B';
                    const k = 1024;
                    const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
                    const i = Math.floor(Math.log(bytes) / Math.log(k));
                    return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
                }}

                async function selectItem(item) {{
                    selectedFile.value = item;
                    fileContent.value = ''; // Clear previous content
                    loading.value = true;
                    error.value = null;

                    try {{
                        const res = await fetch(`/api/content?path=${{encodeURIComponent(item.path)}}`);
                        if (!res.ok) {{
                            const data = await res.json();
                            throw new Error(data.error || 'Failed to load content');
                        }}
                        const data = await res.json();
                        fileContent.value = data.content;
                    }} catch (e) {{
                        error.value = e.message;
                    }} finally {{
                        loading.value = false;
                    }}
                }}

                async function loadRoot() {{
                    loading.value = true;
                    try {{
                        const res = await fetch('/api/tree?path=/');
                        if (!res.ok) throw new Error('Failed to load root');
                        const data = await res.json();
                        // Enrich items with path
                        tree.value = data.items.map(i => ({{ ...i, path: '/' + i.name }}));
                    }} catch (e) {{
                        error.value = "Failed to load directory structure";
                        console.error(e);
                    }} finally {{
                        loading.value = false;
                    }}
                }}

                // Upload state
                const showUpload = ref(false);
                const isDragging = ref(false);
                const uploadSelectedFile = ref(null);
                const uploading = ref(false);
                const uploadProgress = ref(0);
                const uploadMessage = ref('');
                const uploadStatusClass = ref('');
                const uploadPassword = ref('');
                const uploadPasswordRequired = window.location.search.includes('upload_password=1');
                const uploadFileInput = ref(null);

                function toggleUpload() {{
                    showUpload.value = !showUpload.value;
                    if (!showUpload.value) {{
                        uploadSelectedFile.value = null;
                        uploadMessage.value = '';
                    }}
                }}

                function triggerFileInput() {{
                    uploadFileInput.value.click();
                }}

                function onDragOver() {{ isDragging.value = true; }}
                function onDragLeave() {{ isDragging.value = false; }}

                function onDrop(e) {{
                    isDragging.value = false;
                    if (e.dataTransfer.files.length > 0) {{
                        uploadSelectedFile.value = e.dataTransfer.files[0];
                    }}
                }}

                function onUploadFileSelected(e) {{
                    if (e.target.files.length > 0) {{
                        uploadSelectedFile.value = e.target.files[0];
                    }}
                }}

                async function startUpload() {{
                    if (!uploadSelectedFile.value || uploading.value) return;
                    uploading.value = true;
                    uploadProgress.value = 0;
                    uploadMessage.value = '';
                    uploadStatusClass.value = '';

                    const formData = new FormData();
                    formData.append('file', uploadSelectedFile.value);
                    if (uploadPasswordRequired && uploadPassword.value) {{
                        formData.append('password', uploadPassword.value);
                    }}

                    try {{
                        const xhr = new XMLHttpRequest();
                        xhr.open('POST', '/api/upload', true);
                        xhr.upload.onprogress = (e) => {{
                            if (e.lengthComputable) {{
                                uploadProgress.value = (e.loaded / e.total) * 100;
                            }}
                        }};
                        const result = await new Promise((resolve, reject) => {{
                            xhr.onload = () => {{
                                if (xhr.status >= 200 && xhr.status < 300) {{
                                    resolve(JSON.parse(xhr.responseText));
                                }} else {{
                                    let msg = 'Upload failed';
                                    try {{ const r = JSON.parse(xhr.responseText); msg = r.error || msg; }} catch(e) {{}}
                                    reject(new Error(msg));
                                }}
                            }};
                            xhr.onerror = () => reject(new Error('Network error'));
                            xhr.send(formData);
                        }});
                        uploadMessage.value = 'Uploaded: ' + result.filename;
                        uploadStatusClass.value = 'success';
                        uploadProgress.value = 100;
                        uploadSelectedFile.value = null;
                    }} catch (e) {{
                        uploadMessage.value = e.message;
                        uploadStatusClass.value = 'error';
                    }} finally {{
                        uploading.value = false;
                    }}
                }}

                onMounted(() => {{
                    loadRoot();
                }});

                watch(fileContent, () => {{
                    if (!isMarkdown.value) {{
                        // Trigger Prism highlight next tick
                        setTimeout(() => Prism.highlightAll(), 0);
                    }}
                }});

                return {{
                    tree,
                    selectedFile,
                    fileContent,
                    loading,
                    error,
                    currentPath,
                    isMarkdown,
                    languageClass,
                    renderedMarkdown,
                    formatSize,
                    selectItem,
                    showUpload, toggleUpload, isDragging,
                    uploadSelectedFile, uploading, uploadProgress,
                    uploadMessage, uploadStatusClass,
                    uploadPassword, uploadPasswordRequired,
                    uploadFileInput, triggerFileInput,
                    onDragOver, onDragLeave, onDrop,
                    onUploadFileSelected, startUpload,
                }};
            }}
        }}).mount('#app');
    </script>
</body>
</html>
"""


from typing import List, Tuple


def generate_multi_share_spa_html(item_count: int) -> str:
    """
    Generate the SPA HTML page for multi-path sharing.

    Differences from generate_spa_html():
    - Title shows "N items" instead of directory name.
    - ZIP download button points to /download/all.zip.
    - File download links are prefixed with /files/.
    - Clicking a file triggers a browser download (no preview panel).
    - No Legacy View button (replaced by list-only view).

    Args:
        item_count: Number of shared top-level items (for title display).

    Returns:
        Full HTML/CSS/JS string for the SPA page.
    """
    title_text = html.escape(f"{item_count} item{'s' if item_count != 1 else ''}")
    return f"""<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Quick Share - {title_text}</title>
    <script src="https://unpkg.com/vue@3/dist/vue.global.js"></script>
    <style>
        :root {{
            --bg: #f5f5f5;
            --white: #fff;
            --border: #ddd;
            --primary: #007bff;
            --text: #333;
            --header-h: 60px;
        }}
        * {{ box-sizing: border-box; margin: 0; padding: 0; }}
        body {{
            font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
            background: var(--bg);
            color: var(--text);
            height: 100vh;
            display: flex;
            flex-direction: column;
        }}
        header {{
            height: var(--header-h);
            background: var(--white);
            border-bottom: 1px solid var(--border);
            display: flex;
            align-items: center;
            padding: 0 20px;
            justify-content: space-between;
            flex-shrink: 0;
        }}
        .brand {{ font-weight: bold; font-size: 1.1rem; display: flex; align-items: center; gap: 8px; }}
        .brand .sub {{ color: #888; font-weight: normal; font-size: 0.95rem; }}
        .btn {{
            padding: 8px 16px; border-radius: 4px; text-decoration: none;
            font-size: 0.9rem; cursor: pointer; border: 1px solid var(--border);
            background: #f8f9fa; color: var(--text);
        }}
        .btn-primary {{ background: var(--primary); color: white; border-color: var(--primary); }}
        .main {{ flex: 1; overflow-y: auto; padding: 20px; }}
        .file-list {{ background: var(--white); border-radius: 6px; border: 1px solid var(--border); overflow: hidden; }}
        .list-item {{
            display: flex; align-items: center; padding: 10px 16px;
            border-bottom: 1px solid var(--border); gap: 10px;
        }}
        .list-item:last-child {{ border-bottom: none; }}
        .list-item:hover {{ background: #f8f9fa; }}
        .item-icon {{ width: 24px; text-align: center; flex-shrink: 0; }}
        .item-name {{ flex: 1; font-size: 0.95rem; }}
        .item-name a {{ color: inherit; text-decoration: none; }}
        .item-name a:hover {{ color: var(--primary); text-decoration: underline; }}
        .item-size {{ color: #888; font-size: 0.85rem; width: 90px; text-align: right; flex-shrink: 0; }}
        .item-children {{ padding-left: 20px; border-top: 1px solid var(--border); }}
        .loading {{ color: #aaa; font-size: 0.85rem; padding: 6px 16px; }}
        .error-banner {{
            color: #dc3545; background: #f8d7da; padding: 12px 16px;
            border-radius: 4px; margin-bottom: 16px;
        }}
        .empty {{ color: #aaa; text-align: center; padding: 40px; }}
        .upload-section {{
            background: var(--white); border-radius: 6px; border: 1px solid var(--border);
            padding: 20px; margin-top: 16px;
        }}
        .upload-section.hidden {{ display: none; }}
        .upload-dropzone {{
            border: 2px dashed #ccc; border-radius: 6px; padding: 24px;
            text-align: center; cursor: pointer; transition: border-color 0.2s;
        }}
        .upload-dropzone:hover, .upload-dropzone.dragover {{ border-color: var(--primary); background: #f0f7ff; }}
        .upload-btn-small {{ margin-left: 8px; }}
        .upload-progress {{ width: 100%; margin: 8px 0; }}
        .upload-status {{ font-size: 0.85rem; margin-top: 4px; }}
        .upload-status.success {{ color: #28a745; }}
        .upload-status.error {{ color: #dc3545; }}
    </style>
</head>
<body>
    <div id="app">
        <header>
            <div class="brand">
                <span>Quick Share</span>
                <span class="sub">/ {title_text}</span>
            </div>
            <div>
                <button class="btn" @click="toggleUpload" v-if="!showUpload">Upload</button>
                <button class="btn" @click="toggleUpload" v-else>Close Upload</button>
                <a href="/download/all.zip" class="btn btn-primary" style="margin-left:8px;">Download All (ZIP)</a>
            </div>
        </header>
        <div class="main">
            <div v-if="error" class="error-banner">{{{{ error }}}}</div>
            <div v-if="loading && items.length === 0" class="empty">Loading...</div>
            <div v-else-if="items.length === 0" class="empty">No items shared.</div>
            <div v-else class="file-list">
                <file-item
                    v-for="item in items"
                    :key="item.name"
                    :item="item"
                    :virtual-path="'/' + item.name"
                ></file-item>
            </div>

            <!-- Upload Section -->
            <div :class="showUpload ? 'upload-section' : 'upload-section hidden'">
                <h3 style="margin-bottom: 12px;">Upload Files</h3>
                <div v-if="uploadPasswordRequired" class="field" style="margin-bottom:12px;">
                    <label style="display:block;margin-bottom:4px;font-size:0.85rem;color:#555;">Upload Password</label>
                    <input type="password" v-model="uploadPassword" placeholder="Enter upload password"
                           style="width:100%;padding:10px 12px;border:1px solid #ccc;border-radius:6px;font-size:0.95rem;" />
                </div>
                <div :class="isDragging ? 'upload-dropzone dragover' : 'upload-dropzone'"
                     @click="triggerFileInput"
                     @dragover.prevent="onDragOver" @dragleave.prevent="onDragLeave"
                     @drop.prevent="onDrop">
                    <p v-if="!selectedFile">Drag &amp; drop files here or click to select</p>
                    <p v-else>{{{{ selectedFile.name }}}} ({{{{ formatSize(selectedFile.size) }}}})</p>
                </div>
                <input type="file" ref="fileInput" @change="onFileSelected" style="display:none" />
                <progress class="upload-progress" :value="uploadProgress" max="100" v-if="uploading"></progress>
                <div :class="['upload-status', uploadStatusClass]" v-if="uploadMessage">{{{{ uploadMessage }}}}</div>
                <button class="btn btn-primary" @click="startUpload" :disabled="!selectedFile || uploading"
                        style="margin-top: 8px;">
                    {{{{ uploading ? 'Uploading...' : 'Upload' }}}}
                </button>
            </div>
        </div>
    </div>

    <script>
        const {{ createApp, ref, onMounted }} = Vue;

        const FileItem = {{
            name: 'FileItem',
            props: ['item', 'virtualPath'],
            setup(props) {{
                const isOpen = ref(false);
                const children = ref([]);
                const isLoading = ref(false);
                const isFolder = props.item.type === 'directory';

                function toggle() {{
                    if (!isFolder) return;
                    isOpen.value = !isOpen.value;
                    if (isOpen.value && children.value.length === 0) {{
                        loadChildren();
                    }}
                }}

                async function loadChildren() {{
                    isLoading.value = true;
                    try {{
                        const res = await fetch('/api/tree?path=' + encodeURIComponent(props.virtualPath));
                        if (!res.ok) throw new Error('Failed to load');
                        const data = await res.json();
                        children.value = data.items.map(c => ({{
                            ...c,
                            _virtualPath: props.virtualPath + '/' + c.name
                        }}));
                    }} catch(e) {{
                        console.error(e);
                    }} finally {{
                        isLoading.value = false;
                    }}
                }}

                function formatSize(bytes) {{
                    if (!bytes) return '-';
                    const k = 1024, sizes = ['B','KB','MB','GB','TB'];
                    const i = Math.floor(Math.log(bytes) / Math.log(k));
                    return (bytes / Math.pow(k, i)).toFixed(1) + ' ' + sizes[i];
                }}

                return {{ isOpen, isFolder, children, isLoading, toggle, formatSize }};
            }},
            template: `
                <div>
                    <div class="list-item" :style="isFolder ? 'cursor:pointer' : ''" @click="isFolder && toggle()">
                        <span class="item-icon">{{{{ isFolder ? (isOpen ? '📂' : '📁') : '📄' }}}}</span>
                        <span class="item-name">
                            <a v-if="!isFolder" :href="'/files' + virtualPath" :download="item.name">{{{{ item.name }}}}</a>
                            <span v-else>{{{{ item.name }}}}</span>
                        </span>
                        <span class="item-size">{{{{ isFolder ? '-' : formatSize(item.size) }}}}</span>
                    </div>
                    <div v-if="isFolder && isOpen" class="item-children">
                        <div v-if="isLoading" class="loading">Loading...</div>
                        <file-item
                            v-else
                            v-for="child in children"
                            :key="child.name"
                            :item="child"
                            :virtual-path="child._virtualPath"
                        ></file-item>
                        <div v-if="!isLoading && children.length === 0" class="loading">(Empty)</div>
                    </div>
                </div>
            `
        }};

        createApp({{
            components: {{ FileItem }},
            setup() {{
                const items = ref([]);
                const loading = ref(false);
                const error = ref(null);

                // Upload state
                const showUpload = ref(false);
                const isDragging = ref(false);
                const selectedFile = ref(null);
                const uploading = ref(false);
                const uploadProgress = ref(0);
                const uploadMessage = ref('');
                const uploadStatusClass = ref('');
                const uploadPassword = ref('');
                const uploadPasswordRequired = window.location.search.includes('upload_password=1');
                const fileInput = ref(null);

                function toggleUpload() {{
                    showUpload.value = !showUpload.value;
                    if (!showUpload.value) {{
                        selectedFile.value = null;
                        uploadMessage.value = '';
                    }}
                }}

                function triggerFileInput() {{
                    fileInput.value.click();
                }}

                function onDragOver() {{
                    isDragging.value = true;
                }}

                function onDragLeave() {{
                    isDragging.value = false;
                }}

                function onDrop(e) {{
                    isDragging.value = false;
                    const files = e.dataTransfer.files;
                    if (files.length > 0) {{
                        selectedFile.value = files[0];
                    }}
                }}

                function onFileSelected(e) {{
                    const files = e.target.files;
                    if (files.length > 0) {{
                        selectedFile.value = files[0];
                    }}
                }}

                function formatSize(bytes) {{
                    if (!bytes) return '0 B';
                    const k = 1024, sizes = ['B','KB','MB','GB','TB'];
                    const i = Math.floor(Math.log(bytes) / Math.log(k));
                    return (bytes / Math.pow(k, i)).toFixed(1) + ' ' + sizes[i];
                }}

                async function startUpload() {{
                    if (!selectedFile.value || uploading.value) return;

                    uploading.value = true;
                    uploadProgress.value = 0;
                    uploadMessage.value = '';
                    uploadStatusClass.value = '';

                    const formData = new FormData();
                    formData.append('file', selectedFile.value);
                    if (uploadPasswordRequired && uploadPassword.value) {{
                        formData.append('password', uploadPassword.value);
                    }}

                    try {{
                        const xhr = new XMLHttpRequest();
                        xhr.open('POST', '/api/upload', true);

                        xhr.upload.onprogress = (e) => {{
                            if (e.lengthComputable) {{
                                uploadProgress.value = (e.loaded / e.total) * 100;
                            }}
                        }};

                        const result = await new Promise((resolve, reject) => {{
                            xhr.onload = () => {{
                                if (xhr.status >= 200 && xhr.status < 300) {{
                                    resolve(JSON.parse(xhr.responseText));
                                }} else {{
                                    let msg = 'Upload failed';
                                    try {{ const r = JSON.parse(xhr.responseText); msg = r.error || msg; }} catch(e) {{}}
                                    reject(new Error(msg));
                                }}
                            }};
                            xhr.onerror = () => reject(new Error('Network error'));
                            xhr.send(formData);
                        }});

                        uploadMessage.value = 'Uploaded: ' + result.filename;
                        uploadStatusClass.value = 'success';
                        uploadProgress.value = 100;
                        selectedFile.value = null;
                    }} catch (e) {{
                        uploadMessage.value = e.message;
                        uploadStatusClass.value = 'error';
                    }} finally {{
                        uploading.value = false;
                    }}
                }}

                async function loadRoot() {{
                    loading.value = true;
                    try {{
                        const res = await fetch('/api/tree?path=/');
                        if (!res.ok) throw new Error('Failed to load root');
                        const data = await res.json();
                        items.value = data.items;
                    }} catch(e) {{
                        error.value = 'Failed to load file list: ' + e.message;
                    }} finally {{
                        loading.value = false;
                    }}
                }}

                onMounted(loadRoot);
                return {{
                    items, loading, error,
                    showUpload, toggleUpload, isDragging,
                    selectedFile, uploading, uploadProgress,
                    uploadMessage, uploadStatusClass,
                    uploadPassword, uploadPasswordRequired, fileInput,
                    triggerFileInput, onDragOver, onDragLeave,
                    onDrop, onFileSelected, formatSize, startUpload,
                }};
            }}
        }}).mount('#app');
    </script>
</body>
</html>
"""


def generate_multi_share_legacy_html(
    paths: List[Tuple[str, str]],
) -> str:
    """
    Generate server-side rendered HTML for multi-path sharing (legacy mode).

    Used when the server is started with --legacy or ?legacy=1 is in the URL.

    Args:
        paths: List of (abs_path, path_type) tuples.

    Returns:
        HTML string with a table listing all shared items.
    """
    item_count = len(paths)
    title_text = html.escape(f"{item_count} item{'s' if item_count != 1 else ''}")

    rows_html = ""
    for abs_path, path_type in paths:
        name = html.escape(os.path.basename(abs_path))
        if path_type == "file":
            try:
                from .directory_handler import format_file_size
            except ImportError:
                from directory_handler import format_file_size
            try:
                size_str = format_file_size(os.path.getsize(abs_path))
            except OSError:
                size_str = "?"
            type_str = "File"
            action = f'<a href="/files/{name}" download="{name}">Download</a>'
        else:
            size_str = "-"
            type_str = "Directory"
            action = f'<a href="/download/all.zip">Download ZIP</a>'

        rows_html += (
            f"<tr>"
            f"<td>{'📁' if path_type == 'directory' else '📄'} {name}</td>"
            f"<td>{type_str}</td>"
            f"<td>{size_str}</td>"
            f"<td>{action}</td>"
            f"</tr>\n"
        )

    return f"""<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Quick Share - {title_text}</title>
    <style>
        body {{ font-family: Arial, sans-serif; margin: 20px; background: #f5f5f5; }}
        .container {{ max-width: 900px; margin: 0 auto; background: white; padding: 20px; border-radius: 8px; }}
        h1 {{ color: #333; border-bottom: 2px solid #007bff; padding-bottom: 10px; }}
        .btn {{ padding: 10px 20px; background: #007bff; color: white; text-decoration: none; border-radius: 4px; display: inline-block; margin-bottom: 16px; }}
        table {{ width: 100%; border-collapse: collapse; margin-top: 16px; }}
        th, td {{ text-align: left; padding: 10px 12px; border-bottom: 1px solid #ddd; }}
        th {{ background: #f8f9fa; font-weight: 600; }}
        a {{ color: #007bff; text-decoration: none; }}
        a:hover {{ text-decoration: underline; }}
    </style>
</head>
<body>
    <div class="container">
        <h1>Quick Share - {title_text}</h1>
        <a href="/download/all.zip" class="btn">Download All (ZIP)</a>
        <table>
            <thead>
                <tr><th>Name</th><th>Type</th><th>Size</th><th>Action</th></tr>
            </thead>
            <tbody>
                {rows_html}
            </tbody>
        </table>
    </div>
</body>
</html>
"""


def generate_upload_page(upload_password_set: bool = False) -> str:
    """Generate pure HTML5 upload page for standalone upload mode.

    Args:
        upload_password_set: If True, include a password input field.

    Returns:
        Full HTML page string with drag-drop, file selection, progress bar.
    """
    password_section = ""
    if upload_password_set:
        password_section = """
                <div class="field">
                    <label for="password">Upload Password</label>
                    <input type="password" id="password" placeholder="Enter upload password" />
                </div>
            """

    return f"""<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Quick Share - Upload</title>
    <style>
        * {{ box-sizing: border-box; margin: 0; padding: 0; }}
        body {{
            font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
            background: #f5f5f5; color: #333; min-height: 100vh;
            display: flex; align-items: center; justify-content: center;
        }}
        .container {{
            max-width: 520px; width: 100%; margin: 20px;
            background: #fff; border-radius: 8px; padding: 32px;
            box-shadow: 0 2px 12px rgba(0,0,0,0.08);
        }}
        h1 {{ font-size: 1.5rem; margin-bottom: 8px; }}
        .subtitle {{ color: #888; font-size: 0.9rem; margin-bottom: 24px; }}
        #dropzone {{
            border: 2px dashed #ccc; border-radius: 8px; padding: 40px 20px;
            text-align: center; cursor: pointer; transition: border-color 0.2s;
            margin-bottom: 16px;
        }}
        #dropzone:hover, #dropzone.dragover {{ border-color: #007bff; background: #f0f7ff; }}
        #dropzone.has-file {{ border-color: #28a745; }}
        #fileInfo {{ color: #666; font-size: 0.85rem; margin-top: 8px; }}
        .btn-primary {{
            display: block; width: 100%; padding: 12px;
            background: #007bff; color: #fff; border: none; border-radius: 6px;
            font-size: 1rem; cursor: pointer;
        }}
        .btn-primary:disabled {{ opacity: 0.5; cursor: not-allowed; }}
        #progress {{ width: 100%; margin: 12px 0; display: none; }}
        #status {{ margin: 12px 0; font-size: 0.9rem; }}
        .status-success {{ color: #28a745; }}
        .status-error {{ color: #dc3545; }}
        .field {{ margin-bottom: 16px; }}
        .field label {{ display: block; margin-bottom: 4px; font-size: 0.85rem; color: #555; }}
        .field input[type="password"] {{
            width: 100%; padding: 10px 12px; border: 1px solid #ccc;
            border-radius: 6px; font-size: 0.95rem;
        }}
        input[type="file"] {{ display: none; }}
    </style>
</head>
<body>
    <div class="container">
        <h1>Upload Files</h1>
        <p class="subtitle">Select or drag files to upload</p>

        {password_section}

        <div id="dropzone" onclick="document.getElementById('fileInput').click()">
            <p style="font-size: 2rem; margin-bottom: 8px;">+</p>
            <p>Drag &amp; drop files here</p>
            <p style="font-size:0.85rem;color:#888;">or click to select</p>
            <div id="fileInfo"></div>
        </div>

        <input type="file" id="fileInput" multiple />

        <progress id="progress" value="0" max="100"></progress>
        <div id="status"></div>

        <button class="btn-primary" id="uploadBtn" disabled>Upload</button>
    </div>

    <script>
        const dropzone = document.getElementById('dropzone');
        const fileInput = document.getElementById('fileInput');
        const fileInfo = document.getElementById('fileInfo');
        const uploadBtn = document.getElementById('uploadBtn');
        const progress = document.getElementById('progress');
        const status = document.getElementById('status');
        const passwordInput = document.getElementById('password');

        let selectedFiles = [];

        dropzone.addEventListener('dragover', e => {{
            e.preventDefault(); dropzone.classList.add('dragover');
        }});
        dropzone.addEventListener('dragleave', () => dropzone.classList.remove('dragover'));
        dropzone.addEventListener('drop', e => {{
            e.preventDefault(); dropzone.classList.remove('dragover');
            handleFiles(e.dataTransfer.files);
        }});
        fileInput.addEventListener('change', () => handleFiles(fileInput.files));

        function handleFiles(files) {{
            if (!files.length) return;
            selectedFiles = Array.from(files);
            const names = selectedFiles.map(f => f.name).join(', ');
            fileInfo.textContent = selectedFiles.length + ' file(s): ' + names;
            dropzone.classList.add('has-file');
            uploadBtn.disabled = false;
        }}

        uploadBtn.addEventListener('click', uploadFiles);

        async function uploadFiles() {{
            const formData = new FormData();
            selectedFiles.forEach(f => formData.append('file', f));
            if (passwordInput) {{
                formData.append('password', passwordInput.value);
            }}

            uploadBtn.disabled = true;
            progress.style.display = 'block';
            progress.value = 0;
            status.textContent = 'Uploading...';
            status.className = '';

            const xhr = new XMLHttpRequest();
            xhr.open('POST', '/upload', true);

            xhr.upload.onprogress = function(e) {{
                if (e.lengthComputable) {{
                    progress.value = (e.loaded / e.total) * 100;
                }}
            }};

            xhr.onload = function() {{
                if (xhr.status === 200) {{
                    const resp = JSON.parse(xhr.responseText);
                    status.textContent = 'Upload complete: ' + resp.filename;
                    status.className = 'status-success';
                    progress.value = 100;
                    selectedFiles = [];
                    dropzone.classList.remove('has-file');
                    fileInfo.textContent = '';
                }} else {{
                    let msg = 'Upload failed';
                    try {{
                        const resp = JSON.parse(xhr.responseText);
                        msg = resp.error || msg;
                    }} catch(e) {{}}
                    status.textContent = msg;
                    status.className = 'status-error';
                }}
                uploadBtn.disabled = false;
            }};

            xhr.onerror = function() {{
                status.textContent = 'Network error';
                status.className = 'status-error';
                uploadBtn.disabled = false;
            }};

            xhr.send(formData);
        }}
    </script>
</body>
</html>
"""
