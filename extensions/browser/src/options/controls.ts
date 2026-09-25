/** Form rows for the options page: grouped cards of switch rows, editable chip lists, steppers. */

import { h, iconButton, tr } from '../shared-ui/dom.ts';
import { icon, type IconName } from '../shared-ui/icons.ts';

let idCounter = 0;
export function nextId(prefix: string): string {
  idCounter += 1;
  return `${prefix}-${idCounter}`;
}

/** A titled group: optional heading and description, then a frosted card holding the rows. */
export function group(rows: HTMLElement[], title?: string, desc?: string): HTMLElement[] {
  const out: HTMLElement[] = [];
  let headingId: string | undefined;
  if (title) {
    headingId = nextId('group');
    out.push(h('h2', { class: 'group-title', text: title, attrs: { id: headingId } }));
  }
  if (desc) out.push(h('p', { class: 'group-desc', text: desc }));
  const card = h('div', { class: 'group card', attrs: headingId ? { role: 'group', 'aria-labelledby': headingId } : {} }, rows);
  out.push(card);
  return out;
}

export function note(text: string, iconName: IconName = 'info', warning = false): HTMLElement {
  return h('div', { class: `note${warning ? ' is-warning' : ''}` }, [icon(iconName), h('div', { text })]);
}

function textBlock(label: string, hint: string | undefined, forId?: string, hintId?: string): HTMLElement {
  const labelEl = forId
    ? h('label', { class: 'opt-label', text: label, attrs: { for: forId } })
    : h('span', { class: 'opt-label', text: label });
  return h('div', { class: 'opt-text' }, [
    labelEl,
    hint ? h('span', { class: 'opt-hint', text: hint, attrs: hintId ? { id: hintId } : {} }) : null,
  ]);
}

export interface SwitchRowOptions {
  label: string;
  hint?: string;
  checked: boolean;
  disabled?: boolean;
  onChange(checked: boolean): void;
}

export function switchRow(opts: SwitchRowOptions): HTMLElement {
  const id = nextId('switch');
  const hintId = opts.hint ? `${id}-hint` : undefined;
  const input = h('input', {
    class: 'switch',
    attrs: { type: 'checkbox', role: 'switch', id, ...(hintId ? { 'aria-describedby': hintId } : {}) },
  });
  input.checked = opts.checked;
  input.disabled = opts.disabled ?? false;
  input.addEventListener('change', () => opts.onChange(input.checked));
  return h('div', { class: `opt-row${opts.disabled ? ' is-disabled' : ''}` }, [textBlock(opts.label, opts.hint, id, hintId), input]);
}

export function valueRow(label: string, value: string | Node, hint?: string, muted = false): HTMLElement {
  const content = typeof value === 'string' ? h('span', { class: `value${muted ? ' is-muted' : ''}`, text: value }) : value;
  return h('div', { class: 'opt-row' }, [textBlock(label, hint), content]);
}

export function controlRow(label: string, control: HTMLElement, hint?: string, forId?: string): HTMLElement {
  return h('div', { class: 'opt-row' }, [textBlock(label, hint, forId), control]);
}

export interface StepperOptions {
  label: string;
  hint?: string;
  value: number;
  unit: string;
  min?: number;
  step?: number;
  onChange(value: number): void;
}

export function stepperRow(opts: StepperOptions): HTMLElement {
  const id = nextId('stepper');
  const min = opts.min ?? 0;
  const step = opts.step ?? 1;
  const input = h('input', { attrs: { id, type: 'number', min: String(min), step: String(step), inputmode: 'numeric' } });
  input.value = String(opts.value);
  const commit = (): void => {
    const next = Math.max(min, Math.round(Number(input.value) || 0));
    input.value = String(next);
    opts.onChange(next);
  };
  input.addEventListener('change', commit);
  const nudge = (delta: number): void => {
    input.value = String(Math.max(min, (Number(input.value) || 0) + delta));
    commit();
  };
  const minus = h('button', { text: '−', attrs: { type: 'button', 'aria-label': tr('actionDecrease', 'Decrease') } });
  minus.addEventListener('click', () => nudge(-step));
  const plus = h('button', { text: '+', attrs: { type: 'button', 'aria-label': tr('actionIncrease', 'Increase') } });
  plus.addEventListener('click', () => nudge(step));
  const stepper = h('div', { class: 'stepper' }, [input, h('span', { class: 'unit', text: opts.unit }), minus, plus]);
  return controlRow(opts.label, stepper, opts.hint, id);
}

export interface ChipEditorOptions {
  label: string;
  hint?: string;
  values: string[];
  placeholder: string;
  emptyText: string;
  mono?: boolean;
  /** Clean up what was typed; return `null` to reject it. */
  normalize(raw: string): string | null;
  onChange(values: string[]): void;
  extra?: HTMLElement;
}

export function chipEditor(opts: ChipEditorOptions): HTMLElement {
  let values = [...opts.values];
  const inputId = nextId('chips');
  const chips = h('div', { class: `chips${opts.mono ? ' is-mono' : ''}`, attrs: { role: 'list' } });
  const input = h('input', {
    class: 'field',
    attrs: { id: inputId, type: 'text', placeholder: opts.placeholder, autocomplete: 'off', spellcheck: 'false' },
  });
  const status = h('span', { class: 'sr-only', attrs: { role: 'status', 'aria-live': 'polite' } });

  const paint = (): void => {
    if (values.length === 0) {
      chips.replaceChildren(h('span', { class: 'chips-empty', text: opts.emptyText }));
      return;
    }
    chips.replaceChildren(
      ...values.map((value) => {
        const remove = iconButton('close', tr('removeItem', 'Remove $1', value), () => {
          values = values.filter((v) => v !== value);
          opts.onChange(values);
          paint();
          status.textContent = tr('removedItem', 'Removed $1', value);
          input.focus();
        }, '');
        remove.removeAttribute('title');
        return h('span', { class: 'chip', attrs: { role: 'listitem' } }, [h('span', { text: value }), remove]);
      }),
    );
  };

  const add = (): void => {
    const parts = input.value.split(/[\s,]+/).map((p) => p.trim()).filter(Boolean);
    const added: string[] = [];
    for (const part of parts) {
      const value = opts.normalize(part);
      if (value && !values.includes(value) && !added.includes(value)) added.push(value);
    }
    if (added.length === 0) {
      if (parts.length > 0) input.setAttribute('aria-invalid', 'true');
      return;
    }
    input.removeAttribute('aria-invalid');
    values = [...values, ...added];
    input.value = '';
    opts.onChange(values);
    paint();
    status.textContent = tr('addedItem', 'Added $1', added.join(', '));
  };

  input.addEventListener('keydown', (event) => {
    if (event.key === 'Enter') {
      event.preventDefault();
      add();
    }
  });
  input.addEventListener('input', () => input.removeAttribute('aria-invalid'));
  const addBtn = h('button', { class: 'btn', attrs: { type: 'button' } }, [icon('plus'), h('span', { text: tr('actionAdd', 'Add') })]);
  addBtn.addEventListener('click', add);

  paint();
  return h('div', { class: 'opt-block' }, [
    h('div', { class: 'opt-text' }, [
      h('label', { class: 'opt-label', text: opts.label, attrs: { for: inputId } }),
      opts.hint ? h('span', { class: 'opt-hint', text: opts.hint }) : null,
    ]),
    chips,
    h('div', { class: 'inline-add' }, [input, addBtn, opts.extra ?? null]),
    status,
  ]);
}

/** A radio group drawn as a segmented control. */
export function segmented<T extends string>(
  name: string,
  label: string,
  options: Array<{ value: T; label: string; icon?: IconName }>,
  value: T,
  onChange: (value: T) => void,
): HTMLElement {
  const wrap = h('div', { class: 'segmented', attrs: { role: 'radiogroup', 'aria-label': label } });
  for (const option of options) {
    const input = h('input', { attrs: { type: 'radio', name, value: option.value } });
    input.checked = option.value === value;
    input.addEventListener('change', () => {
      if (input.checked) onChange(option.value);
    });
    wrap.append(h('label', {}, [input, option.icon ? icon(option.icon) : null, h('span', { text: option.label })]));
  }
  return wrap;
}
