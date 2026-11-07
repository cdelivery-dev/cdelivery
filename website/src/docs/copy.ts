export {};

const labels = document.body.dataset;
const copyIcon = document.querySelector<HTMLTemplateElement>('#docs-copy-icon')!.innerHTML;
const checkIcon = document.querySelector<HTMLTemplateElement>('#docs-check-icon')!.innerHTML;
const status = document.querySelector<HTMLElement>('#docs-copy-status')!;

document.querySelectorAll<HTMLElement>('pre.astro-code').forEach((pre) => {
  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'nb-code-copy';
  button.setAttribute('aria-label', labels.copyCode!);
  button.innerHTML = copyIcon;
  let reset: ReturnType<typeof setTimeout>;
  button.addEventListener('click', async () => {
    try {
      await navigator.clipboard.writeText(pre.querySelector('code')!.textContent ?? '');
      button.innerHTML = checkIcon;
      button.dataset.state = 'copied';
      button.setAttribute('aria-label', labels.copied!);
      status.textContent = labels.copied!;
    } catch {
      status.textContent = labels.copyFailed!;
    }
    clearTimeout(reset);
    reset = setTimeout(() => {
      button.innerHTML = copyIcon;
      delete button.dataset.state;
      button.setAttribute('aria-label', labels.copyCode!);
      status.textContent = '';
    }, 1500);
  });
  const host = pre.closest<HTMLElement>('.nb-code-figure') ?? pre;
  host.style.position = 'relative';
  host.append(button);
});

document.querySelectorAll<HTMLElement>('.docs-content :is(h2, h3, h4)[id]').forEach((heading) => {
  const link = document.createElement('a');
  link.href = `#${heading.id}`;
  link.className = 'heading-anchor';
  link.textContent = '#';
  link.setAttribute('aria-label', `${labels.copyLink}: ${heading.textContent}`);
  link.addEventListener('click', () => {
    navigator.clipboard.writeText(link.href).then(
      () => { status.textContent = labels.copied!; },
      () => { status.textContent = labels.copyFailed!; },
    );
  });
  heading.append(link);
});
