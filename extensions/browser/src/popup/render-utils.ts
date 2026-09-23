/** Small shared DOM-building helpers for the popup's list-of-items tabs. Plain DOM, no framework. */

export interface RowAction {
  label: string;
  primary?: boolean;
  disabled?: boolean;
  ariaLabel?: string;
  onClick(): void | Promise<void>;
}

export interface RowBadge {
  text: string;
  protectedStyle?: boolean;
}

export interface VariantSelect {
  options: { value: string; label: string }[];
  ariaLabel: string;
  onChange(value: string): void;
}

export interface RowOptions {
  title: string;
  meta: string[];
  badges?: RowBadge[];
  actions: RowAction[];
  variantSelect?: VariantSelect;
  progress?: { fraction: number } | undefined;
}

export function renderItemRow(opts: RowOptions): HTMLLIElement {
  const li = document.createElement('li');
  li.className = 'item';

  const title = document.createElement('div');
  title.className = 'item-title';
  title.textContent = opts.title;
  title.title = opts.title;
  li.appendChild(title);

  const meta = document.createElement('div');
  meta.className = 'item-meta';
  for (const badge of opts.badges ?? []) {
    const span = document.createElement('span');
    span.className = badge.protectedStyle ? 'badge protected' : 'badge';
    span.textContent = badge.text;
    meta.appendChild(span);
  }
  for (const m of opts.meta) {
    const span = document.createElement('span');
    span.textContent = m;
    meta.appendChild(span);
  }
  li.appendChild(meta);

  if (opts.progress) {
    const bar = document.createElement('div');
    bar.className = 'progress-bar';
    const fill = document.createElement('span');
    fill.style.width = `${Math.max(0, Math.min(100, opts.progress.fraction * 100))}%`;
    bar.appendChild(fill);
    li.appendChild(bar);
  }

  const actions = document.createElement('div');
  actions.className = 'item-actions';

  if (opts.variantSelect) {
    const select = document.createElement('select');
    select.setAttribute('aria-label', opts.variantSelect.ariaLabel);
    for (const option of opts.variantSelect.options) {
      const optionEl = document.createElement('option');
      optionEl.value = option.value;
      optionEl.textContent = option.label;
      select.appendChild(optionEl);
    }
    select.addEventListener('change', () => opts.variantSelect?.onChange(select.value));
    actions.appendChild(select);
  }

  for (const action of opts.actions) {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.textContent = action.label;
    if (action.ariaLabel) btn.setAttribute('aria-label', action.ariaLabel);
    if (action.primary) btn.classList.add('primary');
    if (action.disabled) btn.disabled = true;
    btn.addEventListener('click', () => {
      void action.onClick();
    });
    actions.appendChild(btn);
  }
  li.appendChild(actions);

  return li;
}

export function renderEmptyState(container: HTMLElement, message: string): void {
  container.replaceChildren();
  const p = document.createElement('p');
  p.className = 'empty-state';
  p.textContent = message;
  container.appendChild(p);
}

export function renderList(container: HTMLElement, rows: HTMLLIElement[]): void {
  container.replaceChildren();
  const ul = document.createElement('ul');
  ul.className = 'item-list';
  for (const row of rows) ul.appendChild(row);
  container.appendChild(ul);
}
