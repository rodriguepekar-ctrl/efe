// EFE Playground — API client (vanilla JS, no deps).

const API = (() => {
    let token = localStorage.getItem('efe-playground-token') || null;
    let identity = null;

    function setToken(t) {
        token = t;
        if (t) localStorage.setItem('efe-playground-token', t);
        else localStorage.removeItem('efe-playground-token');
    }

    function getToken() { return token; }

    async function call(path, opts = {}) {
        const headers = { 'Content-Type': 'application/json', ...(opts.headers || {}) };
        if (token) headers['Authorization'] = 'Bearer ' + token;
        const resp = await fetch(path, {
            method: opts.method || 'GET',
            headers,
            body: opts.body ? JSON.stringify(opts.body) : undefined,
        });
        const ct = resp.headers.get('content-type') || '';
        const data = ct.includes('json') ? await resp.json() : await resp.text();
        if (!resp.ok) {
            const msg = (typeof data === 'object' && data.message) || data.error || resp.statusText;
            const err = new Error(msg || `HTTP ${resp.status}`);
            err.status = resp.status;
            err.data = data;
            throw err;
        }
        return data;
    }

    async function info() { return call('/info'); }
    async function health() { return call('/health'); }

    async function listTenants() { return call('/auth/tenants'); }

    async function issueToken(tenantId, role = 'admin') {
        const data = await call('/auth/token', {
            method: 'POST',
            body: { tenant_id: tenantId, role },
        });
        setToken(data.token);
        return data;
    }

    async function whoami() {
        if (!token) return null;
        try {
            identity = await call('/auth/me');
            return identity;
        } catch (e) {
            if (e.status === 401) {
                setToken(null);
                identity = null;
                return null;
            }
            throw e;
        }
    }

    function signOut() {
        setToken(null);
        identity = null;
    }

    function getIdentity() { return identity; }

    async function listCatalogs() { return call('/catalog'); }

    async function getCatalog(name) { return call(`/catalog/${encodeURIComponent(name)}`); }

    async function registerCatalog(payload) {
        return call('/catalog/register', { method: 'POST', body: payload });
    }

    async function deleteCatalog(name) {
        return call(`/catalog/${encodeURIComponent(name)}`, { method: 'DELETE' });
    }

    async function classify(catalogName, text, topK = 3) {
        return call('/classify', {
            method: 'POST',
            body: { catalog: catalogName, text, top_k: topK },
        });
    }

    async function encode(text) {
        return call('/encode', { method: 'POST', body: { text } });
    }

    async function listAudit(limit = 50) {
        return call(`/audit?limit=${limit}`);
    }

    async function verifyAuditIntegrity() {
        return call('/audit/integrity');
    }

    async function listSamples() { return call('/samples'); }

    async function getSample(id) { return call(`/samples/${encodeURIComponent(id)}`); }

    async function loadSample(id) {
        return call(`/samples/${encodeURIComponent(id)}/load`, { method: 'POST' });
    }

    return {
        setToken, getToken, getIdentity,
        info, health,
        listTenants, issueToken, whoami, signOut,
        listCatalogs, getCatalog, registerCatalog, deleteCatalog,
        classify, encode,
        listAudit, verifyAuditIntegrity,
        listSamples, getSample, loadSample,
    };
})();
