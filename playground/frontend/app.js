// EFE Playground — main app logic (vanilla JS).

const App = (() => {
    let state = {
        info: null,
        identity: null,
        catalogs: [],
        samples: [],
        editingClasses: [],
    };

    // ─── Utilities ───────────────────────────────────────────────

    function $(sel) { return document.querySelector(sel); }
    function $$(sel) { return Array.from(document.querySelectorAll(sel)); }

    function el(tag, attrs = {}, ...children) {
        const e = document.createElement(tag);
        for (const [k, v] of Object.entries(attrs)) {
            if (k === 'class') e.className = v;
            else if (k === 'data') for (const [dk, dv] of Object.entries(v)) e.dataset[dk] = dv;
            else if (k.startsWith('on')) e.addEventListener(k.slice(2).toLowerCase(), v);
            else if (k === 'html') e.innerHTML = v;
            else e.setAttribute(k, v);
        }
        for (const c of children) {
            if (c == null) continue;
            e.appendChild(typeof c === 'string' ? document.createTextNode(c) : c);
        }
        return e;
    }

    function toast(msg, type = 'info') {
        const t = el('div', { class: `toast ${type}` }, msg);
        $('#toast-container').appendChild(t);
        setTimeout(() => t.remove(), 4000);
    }

    function fmtTime(epoch) {
        return new Date(epoch * 1000).toLocaleString();
    }

    // ─── View routing ───────────────────────────────────────────

    function showView(name) {
        $$('.view').forEach(v => v.classList.remove('active'));
        $$('.nav-item').forEach(n => n.classList.remove('active'));
        const view = $(`#view-${name}`);
        if (!view) return;
        view.classList.add('active');
        const navItem = $$(`.nav-item[data-view="${name}"]`)[0];
        if (navItem) navItem.classList.add('active');

        // Lazy-load view data
        if (name === 'overview') refreshOverview();
        if (name === 'auth') renderAuth();
        if (name === 'catalogs') refreshCatalogs();
        if (name === 'classify') refreshClassify();
        if (name === 'samples') refreshSamples();
        if (name === 'audit') refreshAudit();
        if (name === 'dashboard') refreshDashboard();
    }

    // ─── Overview ───────────────────────────────────────────────

    async function refreshOverview() {
        try {
            const info = await API.info();
            state.info = info;
            const status = `${info.embedder.backend} (${info.embedder.dim}-dim, ${info.embedder.device})`;
            $('#overview-backend-status').textContent = status;
            $('#embedder-info').textContent = info.embedder.backend;
        } catch (e) {
            $('#overview-backend-status').textContent = 'unreachable';
            toast('Backend unreachable : ' + e.message, 'error');
        }
    }

    // ─── Auth ──────────────────────────────────────────────────

    async function renderAuth() {
        try {
            const data = await API.listTenants();
            const grid = $('#tenants-grid');
            grid.innerHTML = '';
            for (const t of data.tenants) {
                const card = el('div', { class: 'card card-action', onClick: () => signIn(t.tenant_id) },
                    el('div', { class: 'card-icon' }, '🏢'),
                    el('h3', {}, t.display_name),
                    el('p', {},
                        `Tenant ID: `, el('code', {}, t.tenant_id), ` · Roles: ${t.roles.join(', ')}`
                    )
                );
                grid.appendChild(card);
            }
        } catch (e) {
            toast('Failed to load tenants: ' + e.message, 'error');
        }
        // If already signed in, show identity panel
        const ident = API.getIdentity();
        if (ident) {
            showAuthPanel(ident);
        }
    }

    async function signIn(tenantId) {
        try {
            const data = await API.issueToken(tenantId, 'admin');
            const ident = await API.whoami();
            state.identity = ident;
            updateTenantPill();
            updateAuthRequired();
            showAuthPanel(ident);
            toast(`Signed in as ${tenantId}`, 'success');
        } catch (e) {
            toast('Sign-in failed: ' + e.message, 'error');
        }
    }

    function showAuthPanel(ident) {
        $('#auth-tenant-id').textContent = ident.tenant_id;
        $('#auth-role').textContent = ident.role;
        $('#auth-actor').textContent = ident.actor;
        $('#auth-token').textContent = (API.getToken() || '').slice(0, 20) + '...';
        $('#auth-result').classList.remove('hidden');
    }

    function signOut() {
        API.signOut();
        state.identity = null;
        $('#auth-result').classList.add('hidden');
        updateTenantPill();
        updateAuthRequired();
        toast('Signed out', 'info');
    }

    function updateTenantPill() {
        const pill = $('#tenant-pill');
        const ident = API.getIdentity();
        if (ident) {
            pill.textContent = `${ident.tenant_id} (${ident.role})`;
            pill.classList.add('active');
        } else {
            pill.textContent = 'Not signed in';
            pill.classList.remove('active');
        }
    }

    function updateAuthRequired() {
        const isAuth = API.getIdentity() !== null;
        $$('.auth-required').forEach(e => e.classList.toggle('hidden', isAuth));
        $$('.auth-content').forEach(e => e.classList.toggle('hidden', !isAuth));
    }

    // ─── Catalogs ──────────────────────────────────────────────

    async function refreshCatalogs() {
        const ident = API.getIdentity();
        if (!ident) { updateAuthRequired(); return; }
        try {
            const data = await API.listCatalogs();
            state.catalogs = data.catalogs;
            renderCatalogList();
        } catch (e) {
            toast('Failed to load catalogs: ' + e.message, 'error');
        }
    }

    function renderCatalogList() {
        const list = $('#catalog-list');
        list.innerHTML = '';
        if (state.catalogs.length === 0) {
            list.appendChild(el('div', { class: 'callout callout-info' },
                'No catalogs yet. Click ', el('strong', {}, '＋ New catalog'), ' to create one, or load a ',
                el('a', { href: '#', onClick: (e) => { e.preventDefault(); showView('samples'); } }, 'sample'),
                '.'
            ));
            return;
        }
        for (const c of state.catalogs) {
            const card = el('div', { class: 'card' },
                el('h3', {}, c.name),
                el('p', {}, `Latest version : v${c.latest_version} · ${c.class_count} classes`),
                el('p', { class: 'audit-event-meta' },
                    'Fingerprint: ', el('code', {}, c.fingerprint.slice(0, 16) + '...')
                ),
                el('div', { class: 'toolbar' },
                    el('button', {
                        class: 'btn btn-secondary',
                        onClick: () => viewCatalog(c.name),
                    }, '👁 View'),
                    el('button', {
                        class: 'btn btn-secondary',
                        onClick: () => useInClassify(c.name),
                    }, '🎯 Classify'),
                    el('button', {
                        class: 'btn btn-danger',
                        onClick: () => deleteCatalog(c.name),
                    }, '🗑 Delete'),
                )
            );
            list.appendChild(card);
        }
    }

    async function viewCatalog(name) {
        try {
            const data = await API.getCatalog(name);
            const json = JSON.stringify(data, null, 2);
            const editor = $('#catalog-editor');
            $('#catalog-editor-title').textContent = `Catalog: ${name}`;
            $('#catalog-edit-name').value = name;
            $('#catalog-edit-name').disabled = true;
            state.editingClasses = data.classes.map(c => ({
                name: c.name,
                examples: c.examples.join('\n'),
                confidence_threshold: c.confidence_threshold,
            }));
            renderEditingClasses();
            editor.classList.remove('hidden');
        } catch (e) {
            toast('Failed: ' + e.message, 'error');
        }
    }

    async function deleteCatalog(name) {
        if (!confirm(`Delete catalog "${name}" ?`)) return;
        try {
            await API.deleteCatalog(name);
            toast(`Deleted ${name}`, 'success');
            refreshCatalogs();
        } catch (e) {
            toast('Delete failed: ' + e.message, 'error');
        }
    }

    function startNewCatalog() {
        state.editingClasses = [
            { name: 'class-a', examples: 'example sentence A1\nexample sentence A2', confidence_threshold: 0.5 },
            { name: 'class-b', examples: 'example sentence B1\nexample sentence B2', confidence_threshold: 0.5 },
        ];
        $('#catalog-edit-name').value = '';
        $('#catalog-edit-name').disabled = false;
        $('#catalog-editor-title').textContent = 'New catalog';
        $('#catalog-result').classList.add('hidden');
        renderEditingClasses();
        $('#catalog-editor').classList.remove('hidden');
    }

    function renderEditingClasses() {
        const list = $('#catalog-classes-list');
        list.innerHTML = '';
        state.editingClasses.forEach((cls, idx) => {
            const row = el('div', { class: 'class-row' },
                el('div', { class: 'class-row-header' },
                    el('span', { class: 'class-row-title' }, `Class ${idx + 1}`),
                    el('button', {
                        class: 'btn btn-danger',
                        onClick: () => {
                            state.editingClasses.splice(idx, 1);
                            renderEditingClasses();
                        },
                    }, '✕ Remove')
                ),
                el('div', { class: 'form-row' },
                    el('label', {}, 'Class name'),
                    el('input', {
                        type: 'text',
                        value: cls.name,
                        onInput: (e) => { state.editingClasses[idx].name = e.target.value; },
                    })
                ),
                el('div', { class: 'form-row' },
                    el('label', {}, 'Example sentences (one per line, ≥ 1)'),
                    (() => {
                        const ta = el('textarea', { rows: 4 });
                        ta.value = cls.examples;
                        ta.addEventListener('input', (e) => {
                            state.editingClasses[idx].examples = e.target.value;
                        });
                        return ta;
                    })()
                ),
                el('div', { class: 'form-row' },
                    el('label', {}, `Confidence threshold (0.0 - 1.0) — current: ${cls.confidence_threshold}`),
                    el('input', {
                        type: 'number',
                        step: '0.01',
                        min: '0',
                        max: '1',
                        value: cls.confidence_threshold,
                        onInput: (e) => { state.editingClasses[idx].confidence_threshold = parseFloat(e.target.value); },
                    })
                )
            );
            list.appendChild(row);
        });
    }

    function addEditingClass() {
        state.editingClasses.push({
            name: `class-${state.editingClasses.length + 1}`,
            examples: 'first example sentence',
            confidence_threshold: 0.5,
        });
        renderEditingClasses();
    }

    async function saveCatalog() {
        const name = $('#catalog-edit-name').value.trim();
        if (!name) { toast('Catalog name required', 'error'); return; }
        if (state.editingClasses.length === 0) { toast('At least 1 class required', 'error'); return; }
        const payload = {
            name,
            classes: state.editingClasses.map(c => ({
                name: c.name,
                examples: c.examples.split('\n').map(s => s.trim()).filter(s => s),
                confidence_threshold: c.confidence_threshold,
            })),
        };
        for (const c of payload.classes) {
            if (!c.name || c.examples.length === 0) {
                toast(`Class missing name or examples`, 'error');
                return;
            }
        }
        try {
            const result = await API.registerCatalog(payload);
            $('#catalog-result').textContent = JSON.stringify(result, null, 2);
            $('#catalog-result').classList.remove('hidden');
            toast(`Catalog ${name} v${result.version} registered`, 'success');
            refreshCatalogs();
        } catch (e) {
            toast('Register failed: ' + e.message, 'error');
        }
    }

    function cancelCatalog() {
        $('#catalog-editor').classList.add('hidden');
        state.editingClasses = [];
    }

    function useInClassify(name) {
        showView('classify');
        // Wait for catalog dropdown to populate then select
        setTimeout(() => {
            const sel = $('#classify-catalog');
            for (const opt of sel.options) {
                if (opt.value === name) { sel.value = name; return; }
            }
        }, 100);
    }

    // ─── Classify ─────────────────────────────────────────────

    async function refreshClassify() {
        const ident = API.getIdentity();
        updateAuthRequired();
        if (!ident) return;
        try {
            const data = await API.listCatalogs();
            const sel = $('#classify-catalog');
            sel.innerHTML = '';
            if (data.catalogs.length === 0) {
                sel.innerHTML = '<option value="">(no catalogs — create one or load a sample)</option>';
                return;
            }
            for (const c of data.catalogs) {
                const o = document.createElement('option');
                o.value = c.name;
                o.textContent = `${c.name} (v${c.latest_version}, ${c.class_count} classes)`;
                sel.appendChild(o);
            }
        } catch (e) {
            toast('Failed: ' + e.message, 'error');
        }
    }

    async function doClassify() {
        const catalog = $('#classify-catalog').value;
        const text = $('#classify-text').value.trim();
        const topK = parseInt($('#classify-top-k').value) || 3;
        if (!catalog) { toast('Select a catalog', 'error'); return; }
        if (!text) { toast('Enter text to classify', 'error'); return; }
        try {
            const result = await API.classify(catalog, text, topK);
            renderClassifyResults(result);
        } catch (e) {
            toast('Classify failed: ' + e.message, 'error');
        }
    }

    function renderClassifyResults(result) {
        const bars = $('#classify-bars');
        bars.innerHTML = '';
        for (const r of result.top_k) {
            const pct = Math.max(0, Math.min(100, (r.score + 1) * 50));  // map [-1,1] → [0,100]
            const cls = r.above_threshold ? 'above-threshold' : 'below-threshold';
            const row = el('div', { class: 'bar-row' },
                el('div', { class: 'bar-label', title: r.class_name }, r.class_name),
                el('div', { class: 'bar-track' },
                    el('div', { class: `bar-fill ${cls}`, style: `width: ${pct}%` })
                ),
                el('div', { class: 'bar-score' }, r.score.toFixed(4))
            );
            bars.appendChild(row);
        }
        $('#classify-raw').textContent = JSON.stringify(result, null, 2);
        $('#classify-results').classList.remove('hidden');
    }

    function clearClassify() {
        $('#classify-text').value = '';
        $('#classify-results').classList.add('hidden');
    }

    // ─── Encode ───────────────────────────────────────────────

    async function doEncode() {
        const text = $('#encode-text').value.trim();
        if (!text) { toast('Enter text to encode', 'error'); return; }
        try {
            const result = await API.encode(text);
            $('#encode-dim').textContent = result.dim;
            $('#encode-norm').textContent = result.norm.toFixed(6);
            $('#encode-backend').textContent = result.embedder_info.backend + ' / ' + result.embedder_info.model_name;
            // Visualization : first 32 components
            const bars = el('div', { class: 'vector-bars' });
            const slice = result.embedding.slice(0, 64);
            const max = Math.max(...slice.map(Math.abs));
            for (const v of slice) {
                const h = max > 0 ? Math.abs(v / max) * 100 : 0;
                const cls = v < 0 ? 'vector-bar negative' : 'vector-bar';
                bars.appendChild(el('div', { class: cls, style: `height: ${h}%` }));
            }
            const container = $('#encode-vector-bars');
            container.innerHTML = '';
            container.appendChild(bars);
            $('#encode-raw').textContent = JSON.stringify(result.embedding.slice(0, 100), null, 1) + '\n... (' + (result.embedding.length - 100) + ' more)';
            $('#encode-result').classList.remove('hidden');
        } catch (e) {
            toast('Encode failed: ' + e.message, 'error');
        }
    }

    // ─── Samples ──────────────────────────────────────────────

    async function refreshSamples() {
        const ident = API.getIdentity();
        updateAuthRequired();
        if (!ident) return;
        try {
            const data = await API.listSamples();
            state.samples = data.samples;
            const grid = $('#samples-grid');
            grid.innerHTML = '';
            for (const s of state.samples) {
                const card = el('div', { class: 'card' },
                    el('div', { class: 'card-icon' }, s.language === 'multi' ? '🌍' : '📦'),
                    el('h3', {}, s.name),
                    el('p', {}, s.description),
                    el('p', { class: 'audit-event-meta' },
                        `${s.class_count} classes · language: ${s.language}`
                    ),
                    el('button', {
                        class: 'btn btn-primary',
                        onClick: () => loadSample(s.id),
                    }, '📥 Load into ' + (ident.tenant_id))
                );
                grid.appendChild(card);
            }
        } catch (e) {
            toast('Failed: ' + e.message, 'error');
        }
    }

    async function loadSample(sampleId) {
        try {
            const result = await API.loadSample(sampleId);
            toast(`Loaded ${result.catalog_name} v${result.version}`, 'success');
            // Optionally redirect to classify view
            setTimeout(() => showView('classify'), 800);
        } catch (e) {
            toast('Load failed: ' + e.message, 'error');
        }
    }

    // ─── Audit ────────────────────────────────────────────────

    async function refreshAudit() {
        const ident = API.getIdentity();
        updateAuthRequired();
        if (!ident) return;
        try {
            const data = await API.listAudit(50);
            const list = $('#audit-list');
            list.innerHTML = '';
            if (data.events.length === 0) {
                list.appendChild(el('div', { class: 'callout callout-info' }, 'No audit events yet for this tenant.'));
                return;
            }
            for (const e of data.events) {
                const event = el('div', { class: 'audit-event' },
                    el('div', { class: 'audit-event-header' },
                        el('span', { class: 'audit-event-action' }, e.action),
                        el('span', { class: 'audit-event-time' }, fmtTime(e.timestamp))
                    ),
                    el('div', {}, `${e.actor} → ${e.resource} (${e.outcome})`),
                    el('div', { class: 'audit-event-meta' },
                        `HMAC: ${e.hmac.slice(0, 16)}... · prev: ${e.prev_hmac.slice(0, 16)}...`
                    ),
                    Object.keys(e.metadata || {}).length > 0
                        ? el('div', { class: 'audit-event-meta' }, JSON.stringify(e.metadata))
                        : null
                );
                list.appendChild(event);
            }
        } catch (err) {
            toast('Failed: ' + err.message, 'error');
        }
    }

    async function verifyAuditChain() {
        try {
            const result = await API.verifyAuditIntegrity();
            const div = $('#audit-integrity');
            div.classList.remove('hidden', 'callout-success', 'callout-error');
            if (result.ok) {
                div.classList.add('callout', 'callout-success');
                div.innerHTML = `<strong>✓ Chain OK</strong> — ${result.count} events, all HMAC links valid.`;
            } else {
                div.classList.add('callout', 'callout-error');
                div.innerHTML = `<strong>✗ Chain BROKEN at index ${result.broken_at}</strong> — possible tampering detected.`;
            }
        } catch (e) {
            toast('Verify failed: ' + e.message, 'error');
        }
    }

    // ─── Dashboard ────────────────────────────────────────────

    async function refreshDashboard() {
        const ident = API.getIdentity();
        updateAuthRequired();
        if (!ident) return;
        try {
            const [info, catalogs, audit, integrity] = await Promise.all([
                API.info(),
                API.listCatalogs(),
                API.listAudit(200),
                API.verifyAuditIntegrity(),
            ]);
            $('#metric-catalogs').textContent = catalogs.catalogs.length;
            $('#metric-events').textContent = audit.count;
            $('#metric-integrity').textContent = integrity.ok ? '✓ OK' : '✗ broken';
            $('#metric-integrity').style.color = integrity.ok ? 'var(--color-success)' : 'var(--color-error)';
            $('#metric-backend').textContent = info.embedder.backend;
            $('#dashboard-last-updated').textContent = 'updated ' + new Date().toLocaleTimeString();

            // Action breakdown
            const counts = {};
            for (const e of audit.events) {
                counts[e.action] = (counts[e.action] || 0) + 1;
            }
            const breakdown = $('#action-breakdown');
            breakdown.innerHTML = '';
            const max = Math.max(1, ...Object.values(counts));
            for (const [action, count] of Object.entries(counts).sort((a, b) => b[1] - a[1])) {
                const pct = (count / max) * 100;
                const row = el('div', { class: 'bar-row' },
                    el('div', { class: 'bar-label' }, action),
                    el('div', { class: 'bar-track' },
                        el('div', { class: 'bar-fill', style: `width: ${pct}%` })
                    ),
                    el('div', { class: 'bar-score' }, count.toString())
                );
                breakdown.appendChild(row);
            }
            // Recent activity
            const recent = $('#recent-activity');
            recent.innerHTML = '';
            for (const e of audit.events.slice(0, 5)) {
                recent.appendChild(el('div', { class: 'audit-event' },
                    el('div', { class: 'audit-event-header' },
                        el('span', { class: 'audit-event-action' }, e.action),
                        el('span', { class: 'audit-event-time' }, fmtTime(e.timestamp))
                    ),
                    el('div', {}, `${e.actor} → ${e.resource}`)
                ));
            }
        } catch (e) {
            toast('Dashboard failed: ' + e.message, 'error');
        }
    }

    // ─── Init ─────────────────────────────────────────────────

    async function init() {
        // Sidebar nav
        $$('.nav-item').forEach(btn => {
            btn.addEventListener('click', () => showView(btn.dataset.view));
        });
        // Card-action elements
        document.body.addEventListener('click', (e) => {
            const target = e.target.closest('[data-target]');
            if (target) {
                e.preventDefault();
                showView(target.dataset.target);
            }
        });

        // Auth signout
        $('#auth-signout').addEventListener('click', signOut);

        // Catalog editor
        $('#catalog-new-btn').addEventListener('click', startNewCatalog);
        $('#catalog-refresh-btn').addEventListener('click', refreshCatalogs);
        $('#catalog-add-class-btn').addEventListener('click', addEditingClass);
        $('#catalog-save-btn').addEventListener('click', saveCatalog);
        $('#catalog-cancel-btn').addEventListener('click', cancelCatalog);

        // Classify
        $('#classify-btn').addEventListener('click', doClassify);
        $('#classify-clear-btn').addEventListener('click', clearClassify);

        // Encode
        $('#encode-btn').addEventListener('click', doEncode);

        // Audit
        $('#audit-refresh-btn').addEventListener('click', refreshAudit);
        $('#audit-verify-btn').addEventListener('click', verifyAuditChain);

        // Dashboard
        $('#dashboard-refresh-btn').addEventListener('click', refreshDashboard);

        // Initial state
        await refreshOverview();

        // Restore session if token present
        const ident = await API.whoami();
        if (ident) {
            state.identity = ident;
            updateTenantPill();
        }
        updateAuthRequired();

        // Default view
        showView('overview');
    }

    return { init, showView };
})();

document.addEventListener('DOMContentLoaded', App.init);
