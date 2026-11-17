// Cross-origin isolation for hosts that cannot send headers (GitHub Pages). The threaded browser build needs
// SharedArrayBuffer, which browsers only allow on a cross-origin isolated page (COOP / COEP headers). tools/web/serve.py
// sends them itself; elsewhere index.html registers this worker, which re-serves the page's own responses with them.
// It reads nothing and stores nothing: the game files the player picks never go through a fetch.
self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', (e) => e.waitUntil(self.clients.claim()));
self.addEventListener('fetch', (e) => {
  const req = e.request;
  if (new URL(req.url).origin !== self.location.origin) return;
  if (req.cache === 'only-if-cached' && req.mode !== 'same-origin') return;
  e.respondWith(
    fetch(req).then((res) => {
      if (res.status === 0) return res;
      const headers = new Headers(res.headers);
      headers.set('Cross-Origin-Opener-Policy', 'same-origin');
      headers.set('Cross-Origin-Embedder-Policy', 'require-corp');
      headers.set('Cross-Origin-Resource-Policy', 'same-origin');
      return new Response(res.body, { status: res.status, statusText: res.statusText, headers });
    }),
  );
});
