export {};

for (const panel of document.querySelectorAll<HTMLElement>('[data-copy-panel]')) {
  const install = panel.closest<HTMLElement>('[data-install]');
  const copy = panel.querySelector<HTMLButtonElement>('[data-copy]');
  const content = panel.querySelector<HTMLElement>('[data-copy-text]');
  const status = panel.querySelector<HTMLElement>('[data-copy-status]');
  if (!install || !copy || !content || !status) continue;
  const buttons = install.querySelectorAll<HTMLButtonElement>('[data-copy]');
  let feedbackTimer: number | undefined;
  let attempt = 0;
  const reset = () => {
    attempt++;
    window.clearTimeout(feedbackTimer);
    copy.removeAttribute('data-copied');
    status.textContent = '';
    status.removeAttribute('data-failed');
  };
  install.addEventListener('change', reset);
  copy.hidden = false;
  copy.addEventListener('click', async () => {
    const text = content.textContent?.trim();
    if (!text) return;
    reset();
    const currentAttempt = attempt;
    for (const button of buttons) button.disabled = true;
    try {
      await navigator.clipboard.writeText(text);
      if (currentAttempt !== attempt) return;
      copy.setAttribute('data-copied', '');
      status.textContent = copy.dataset.success ?? '';
      feedbackTimer = window.setTimeout(reset, 2000);
    } catch {
      if (currentAttempt !== attempt) return;
      content.focus();
      const selection = window.getSelection();
      const range = document.createRange();
      range.selectNodeContents(content);
      selection?.removeAllRanges();
      selection?.addRange(range);
      status.setAttribute('data-failed', '');
      status.textContent = copy.dataset.failure ?? '';
    } finally {
      for (const button of buttons) button.disabled = false;
    }
  });
}
