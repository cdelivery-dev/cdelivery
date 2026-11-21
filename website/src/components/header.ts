const header = document.querySelector<HTMLElement>('.site-header');

if (header) {
  const navigation = header.querySelector<HTMLElement>('.site-navigation');
  const mobileToggle = header.querySelector<HTMLButtonElement>('.mobile-toggle');
  const menus = Array.from(header.querySelectorAll<HTMLDetailsElement>('[data-menu]'));
  const desktop = window.matchMedia('(width >= 800px)');
  const hover = window.matchMedia('(hover: hover) and (pointer: fine)');
  let closeTimer: number | undefined;

  function setMenu(menu: HTMLDetailsElement, open = false) {
    if (open) closeMenus(menu);
    const panel = menu.querySelector<HTMLElement>(':scope > div');
    const focused = !open && panel?.contains(document.activeElement);
    menu.open = open;
    if (panel) panel.inert = !open;
    if (focused) menu.querySelector('summary')?.focus();
  }

  function closeMenus(except?: HTMLDetailsElement) {
    window.clearTimeout(closeTimer);
    menus.forEach((menu) => { if (menu !== except) setMenu(menu); });
  }

  function scheduleClose() {
    window.clearTimeout(closeTimer);
    closeTimer = window.setTimeout(() => {
      const menu = navigation?.querySelector<HTMLDetailsElement>('details[open]');
      if (menu && !menu.querySelector('.nav-panel')?.contains(document.activeElement)) setMenu(menu);
    }, 120);
  }

  function closeMobileNavigation() {
    mobileToggle?.setAttribute('aria-expanded', 'false');
  }

  menus.forEach((menu) => {
    menu.addEventListener('toggle', () => setMenu(menu, menu.open));
    menu.querySelector('summary')?.addEventListener('click', (event) => {
      event.preventDefault();
      setMenu(menu, !menu.open);
    });
    const area = menu.closest<HTMLElement>('.nav-item') ?? menu;
    area.addEventListener('focusout', (event) => {
      if (event.relatedTarget instanceof Node && !area.contains(event.relatedTarget)
        && (desktop.matches || !navigation?.contains(event.relatedTarget))) setMenu(menu);
    });
  });

  mobileToggle?.addEventListener('click', () => {
    const open = mobileToggle.getAttribute('aria-expanded') !== 'true';
    closeMenus();
    mobileToggle.setAttribute('aria-expanded', String(open));
    if (open) navigation?.querySelector<HTMLAnchorElement>('a')?.focus();
  });
  navigation?.addEventListener('focusout', (event) => {
    if (event.relatedTarget instanceof Node && event.relatedTarget !== mobileToggle && !navigation.contains(event.relatedTarget)) closeMobileNavigation();
  });
  header.addEventListener('focusout', (event) => {
    const target = event.target;
    if (event.relatedTarget || !(target instanceof HTMLElement)) return;
    if (desktop.matches && target === mobileToggle) navigation?.querySelector<HTMLAnchorElement>('a')?.focus();
    if (!desktop.matches && navigation?.contains(target) && target.getClientRects().length === 0) mobileToggle?.focus();
  });
  header.querySelectorAll<HTMLElement>('.nav-item').forEach((item) => {
    const menu = item.querySelector('details');
    item.addEventListener('pointerenter', (event) => {
      if (!desktop.matches || !hover.matches || event.pointerType !== 'mouse') return;
      if (!menu) return scheduleClose();
      setMenu(menu, true);
    });
    item.addEventListener('pointerleave', (event) => {
      if (menu && desktop.matches && hover.matches && event.pointerType === 'mouse') scheduleClose();
    });
  });
  document.addEventListener('click', (event) => {
    const target = event.target;
    if (!(target instanceof Node)) return;
    menus.forEach((menu) => {
      const area = menu.closest('.nav-item') ?? menu;
      if (!area.contains(target)) setMenu(menu);
    });
    if (!header.contains(target)) closeMobileNavigation();
  });
  document.addEventListener('keydown', (event) => {
    if (event.key !== 'Escape') return;
    const open = menus.find((menu) => menu.open);
    if (open) {
      setMenu(open);
      open.querySelector('summary')?.focus();
    } else if (mobileToggle?.getAttribute('aria-expanded') === 'true') {
      closeMobileNavigation();
      mobileToggle.focus();
    }
  });

  function reset() {
    const focused = document.activeElement;
    closeMenus();
    closeMobileNavigation();
    if (desktop.matches && focused === mobileToggle) navigation?.querySelector<HTMLAnchorElement>('a')?.focus();
    if (!desktop.matches && navigation?.contains(focused)) mobileToggle?.focus();
  }

  reset();
  header.setAttribute('data-menu-ready', '');
  desktop.addEventListener('change', reset);
}
