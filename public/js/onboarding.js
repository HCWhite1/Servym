// Présentation au premier lancement : accueil, démarrage automatique, affichage,
// repères. Chaque choix s'applique tout de suite ; « Passer » garde l'existant.
import { api, can, store } from './lib.js';
import { loadAutostart, projectSwitches, setInstanceAutostart, setProjectAutostart, setTheme } from './settings.js';

const $ = (sel) => document.querySelector(sel);
const dialog = $('#onboarding');
const allSteps = [...dialog.querySelectorAll('.ob-step')];
let steps = allSteps;

let handlers;
let step = 0;

export function init(hs) {
  handlers = hs;
  $('#ob-next').addEventListener('click', () => (step === steps.length - 1 ? finish() : go(step + 1)));
  $('#ob-back').addEventListener('click', () => go(step - 1));
  $('#ob-skip').addEventListener('click', finish);
  // Échap vaut « Passer » : la présentation ne revient pas au prochain chargement.
  dialog.addEventListener('cancel', (e) => {
    e.preventDefault();
    finish();
  });
  $('#ob-autostart').addEventListener('change', (e) => setInstanceAutostart(e.target.checked, e.target));
  $('#ob-projects').addEventListener('change', (e) => setProjectAutostart(e.target.dataset.id, e.target.checked, e.target));
  pick('#ob-theme', setTheme);
  pick('#ob-layout', (value) => {
    handlers.setLayout(value);
    check('#ob-layout', value);
  });
}

function pick(sel, onPick) {
  $(sel).addEventListener('click', (e) => {
    const button = e.target.closest('[data-value]');
    if (button) onPick(button.dataset.value);
  });
}

function check(sel, value) {
  for (const b of $(sel).querySelectorAll('[data-value]')) b.setAttribute('aria-checked', String(b.dataset.value === value));
}

export async function open() {
  // Sans permission sur le démarrage (Servynex) ni sur les projets, l'étape « Démarrage » disparaît.
  steps = allSteps.filter((s) => s.dataset.step !== '2' || can('instance') || can('edit'));
  for (const s of allSteps) s.hidden = true;
  [...$('#ob-dots').children].forEach((dot, i) => (dot.hidden = i >= steps.length));
  $('#ob-projects').replaceChildren(...projectSwitches());
  for (const input of $('#ob-projects').querySelectorAll('input')) input.disabled = !can('edit');
  check('#ob-theme', store.get('theme', 'system'));
  check('#ob-layout', handlers.prefs().layout);
  go(0);
  if (!dialog.open) dialog.showModal();
  $('#ob-next').focus();
  const input = $('#ob-autostart');
  const enabled = await loadAutostart();
  if (enabled !== null) {
    input.checked = enabled;
    input.disabled = !can('instance');
    if (!can('instance')) $('#ob-autostart-hint').textContent = 'Réglé par un administrateur.';
  } else {
    $('#ob-autostart-hint').textContent = 'État de la tâche planifiée illisible : réglable plus tard avec autostart.bat.';
  }
}

function go(n) {
  step = Math.max(0, Math.min(steps.length - 1, n));
  steps.forEach((s, i) => {
    s.hidden = i !== step;
    const kicker = s.querySelector('.ob-kicker');
    if (kicker) kicker.textContent = `Étape ${i} sur ${steps.length - 1}`;
  });
  dialog.setAttribute('aria-labelledby', steps[step].querySelector('h2').id || 'ob-title-1');
  [...$('#ob-dots').children].forEach((dot, i) => dot.classList.toggle('on', i === step));
  $('#ob-back').hidden = step === 0;
  $('#ob-skip').hidden = step === steps.length - 1;
  $('#ob-next').textContent = step === 0 ? 'Commencer' : step === steps.length - 1 ? 'Terminer' : 'Continuer';
}

async function finish() {
  dialog.close();
  await api('instance/onboarded', { method: 'POST' }).catch(() => {});
}
