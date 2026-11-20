(() => {
  const sync = document.querySelector('#sync');
  const report = document.querySelector('#sync-reporte');
  const status = document.querySelector('#sync-estado');
  const sources = document.querySelector('#sync-fuentes');
  if (!sync || !report) return;
  if (location.protocol === 'file:') { sync.disabled = report.disabled = true; return; }
  let active = false;
  let observed = false;
  let localError = null;
  const poll = async () => {
    try {
      const response = await fetch('/api/estado', { cache: 'no-store' });
      if (!response.ok) throw new Error('Servidor no disponible');
      const state = await response.json();
      sync.disabled = report.disabled = state.corriendo;
      status.textContent = state.error ? `Falló: ${state.error}` : localError || (state.corriendo ? `Sincronizando: ${state.etapa}` : 'Listo para sincronizar.');
      sources.replaceChildren(...Object.entries(state.fuentes || {}).map(([name, result]) => {
        const li = document.createElement('li'); li.textContent = `${name}: ${result}`; return li;
      }));
      if (state.corriendo) observed = true;
      if (active && observed && !state.corriendo) { active = false; if (!state.error) location.reload(); }
    } catch (_) { status.textContent = 'No se pudo consultar el servidor. Puedes volver a intentar Sync.'; sync.disabled = report.disabled = false; }
  };
  const run = async (kind) => {
    sync.disabled = report.disabled = true; observed = false;
    localError = null;
    status.textContent = 'Iniciando sincronización…';
    try {
      const response = await fetch(`/api/${kind}`, { method: 'POST', headers: { 'X-Diario': '1', 'Content-Type': 'application/json' }, body: '{}' });
      const result = await response.json();
      if (!response.ok) throw new Error(result.error || 'No se pudo iniciar');
      active = true; observed = true;
      document.querySelector('#sync-detalle').open = true;
      await poll();
    } catch (error) { localError = error.message; await poll(); }
  };
  sync.addEventListener('click', () => run('sync'));
  report.addEventListener('click', () => run('reporte'));
  poll(); setInterval(poll, 1500);
})();
