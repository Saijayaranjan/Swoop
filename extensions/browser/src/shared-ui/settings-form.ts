/**
 * The settings editor shared verbatim between the popup's Settings tab and the full-page options
 * page (`src/options/index.ts`). Edits `ExtensionSettings` — which embeds every
 * `BrowserSettings` field the desktop app knows about (crates/osprey-domain/src/settings.rs) plus
 * the extension-only additions (`always_browser_domains`, `notify_on_complete`/`_failure`) — via
 * `browser.storage.local` (`SettingsStore`).
 */

import { t } from '../shared/i18n.ts';
import { SettingsStore } from '../shared/settings.ts';
import type { ExtensionSettings } from '../shared/types.ts';

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  return node;
}

function renderTagList(
  container: HTMLElement,
  values: string[],
  placeholder: string,
  ariaLabel: string,
  onChange: (next: string[]) => void,
): void {
  container.replaceChildren();

  const list = el('div', 'tag-list');
  for (const value of values) {
    const tag = el('span', 'tag');
    const text = document.createElement('span');
    text.textContent = value;
    tag.appendChild(text);
    const remove = document.createElement('button');
    remove.type = 'button';
    remove.textContent = '×';
    remove.setAttribute('aria-label', `${t('remove') || 'Remove'} ${value}`);
    remove.addEventListener('click', () => onChange(values.filter((v) => v !== value)));
    tag.appendChild(remove);
    list.appendChild(tag);
  }
  container.appendChild(list);

  const input = el('input', '');
  input.type = 'text';
  input.placeholder = placeholder;
  input.setAttribute('aria-label', ariaLabel);
  input.addEventListener('keydown', (event) => {
    if (event.key !== 'Enter') return;
    event.preventDefault();
    const raw = input.value.trim().replace(/^\./, '');
    if (raw.length === 0) return;
    if (!values.includes(raw)) onChange([...values, raw]);
    input.value = '';
  });
  container.appendChild(input);
}

function renderCheckboxRow(
  parent: HTMLElement,
  id: string,
  label: string,
  checked: boolean,
  onChange: (checked: boolean) => void,
): void {
  const row = el('div', 'checkbox-row');
  const input = document.createElement('input');
  input.type = 'checkbox';
  input.id = id;
  input.checked = checked;
  input.addEventListener('change', () => onChange(input.checked));
  const labelEl = document.createElement('label');
  labelEl.htmlFor = id;
  labelEl.textContent = label;
  row.append(input, labelEl);
  parent.appendChild(row);
}

export async function mountSettingsForm(container: HTMLElement, store: SettingsStore): Promise<void> {
  let settings = await store.get();
  let saveTimer: ReturnType<typeof setTimeout> | undefined;

  container.replaceChildren();
  const form = el('div', 'settings-form');
  container.appendChild(form);

  const statusEl = el('span', 'save-status');

  function scheduleSave(): void {
    statusEl.textContent = t('saving') || 'Saving…';
    if (saveTimer !== undefined) clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      store
        .set(settings)
        .then(() => {
          statusEl.textContent = t('saved') || 'Saved';
        })
        .catch(() => {
          statusEl.textContent = t('saveFailed') || 'Save failed';
        });
    }, 250);
  }

  function patch(next: Partial<ExtensionSettings>): void {
    settings = { ...settings, ...next };
    scheduleSave();
  }

  // --- Interception -----------------------------------------------------------------------
  const interceptSection = el('div', 'settings-field');
  renderCheckboxRow(
    interceptSection,
    'opt-intercept',
    t('settingInterceptDownloads') || 'Intercept browser downloads',
    settings.intercept_downloads,
    (checked) => patch({ intercept_downloads: checked }),
  );
  form.appendChild(interceptSection);

  const minSizeField = el('div', 'settings-field');
  const minSizeLabel = document.createElement('label');
  minSizeLabel.htmlFor = 'opt-min-size';
  minSizeLabel.textContent = t('settingMinSize') || 'Minimum size to intercept (MB)';
  const minSizeInput = document.createElement('input');
  minSizeInput.type = 'number';
  minSizeInput.id = 'opt-min-size';
  minSizeInput.min = '0';
  minSizeInput.value = String(Math.round(settings.intercept_min_size / (1024 * 1024)));
  minSizeInput.addEventListener('change', () => {
    const mb = Math.max(0, Number(minSizeInput.value) || 0);
    patch({ intercept_min_size: mb * 1024 * 1024 });
  });
  minSizeField.append(minSizeLabel, minSizeInput);
  form.appendChild(minSizeField);

  const extField = el('div', 'settings-field');
  const extLabel = document.createElement('label');
  extLabel.textContent = t('settingExtensions') || 'File extensions to intercept';
  extField.appendChild(extLabel);
  const extTags = el('div', '');
  extField.appendChild(extTags);
  renderTagList(
    extTags,
    settings.intercept_extensions,
    t('addExtensionPlaceholder') || 'Add extension, press Enter',
    t('settingExtensions') || 'File extensions to intercept',
    (next) => {
      patch({ intercept_extensions: next });
      renderTagList(extTags, next, t('addExtensionPlaceholder') || 'Add extension, press Enter', t('settingExtensions') || 'File extensions to intercept', (n) => {
        patch({ intercept_extensions: n });
      });
    },
  );
  form.appendChild(extField);

  // --- Exclusions ---------------------------------------------------------------------------
  const excludedDomainsField = el('div', 'settings-field');
  const excludedDomainsLabel = document.createElement('label');
  excludedDomainsLabel.textContent = t('settingExcludedDomains') || 'Never intercept on these domains';
  excludedDomainsField.appendChild(excludedDomainsLabel);
  const excludedDomainsTags = el('div', '');
  excludedDomainsField.appendChild(excludedDomainsTags);
  const renderExcludedDomains = (values: string[]): void => {
    renderTagList(
      excludedDomainsTags,
      values,
      t('addDomainPlaceholder') || 'example.com, press Enter',
      t('settingExcludedDomains') || 'Never intercept on these domains',
      (next) => {
        patch({ excluded_domains: next });
        renderExcludedDomains(next);
      },
    );
  };
  renderExcludedDomains(settings.excluded_domains);
  form.appendChild(excludedDomainsField);

  const patternsField = el('div', 'settings-field');
  const patternsLabel = document.createElement('label');
  patternsLabel.textContent = t('settingExcludedPatterns') || 'Never intercept matching URL patterns';
  const patternsHint = el('span', 'hint');
  patternsHint.textContent = t('urlPatternHint') || 'Wildcards (*) allowed.';
  patternsField.append(patternsLabel, patternsHint);
  const patternsTags = el('div', '');
  patternsField.appendChild(patternsTags);
  const renderPatterns = (values: string[]): void => {
    renderTagList(
      patternsTags,
      values,
      t('addPatternPlaceholder') || 'https://*/ads/*, press Enter',
      t('settingExcludedPatterns') || 'Never intercept matching URL patterns',
      (next) => {
        patch({ excluded_url_patterns: next });
        renderPatterns(next);
      },
    );
  };
  renderPatterns(settings.excluded_url_patterns);
  form.appendChild(patternsField);

  const alwaysBrowserField = el('div', 'settings-field');
  const alwaysBrowserLabel = document.createElement('label');
  alwaysBrowserLabel.textContent = t('settingAlwaysBrowser') || 'Always use the browser\'s own download on these sites';
  alwaysBrowserField.appendChild(alwaysBrowserLabel);
  const alwaysBrowserTags = el('div', '');
  alwaysBrowserField.appendChild(alwaysBrowserTags);
  const renderAlwaysBrowser = (values: string[]): void => {
    renderTagList(
      alwaysBrowserTags,
      values,
      t('addDomainPlaceholder') || 'example.com, press Enter',
      t('settingAlwaysBrowser') || "Always use the browser's own download on these sites",
      (next) => {
        patch({ always_browser_domains: next });
        renderAlwaysBrowser(next);
      },
    );
  };
  renderAlwaysBrowser(settings.always_browser_domains);
  form.appendChild(alwaysBrowserField);

  // --- Media & confirmation -------------------------------------------------------------------
  const toggles = el('div', 'settings-field');
  renderCheckboxRow(
    toggles,
    'opt-detect-media',
    t('settingDetectMedia') || 'Detect playable media on pages',
    settings.detect_media,
    (checked) => patch({ detect_media: checked }),
  );
  renderCheckboxRow(
    toggles,
    'opt-confirm',
    t('settingShowConfirmation') || 'Show a confirmation toast when a download is sent',
    settings.show_confirmation,
    (checked) => patch({ show_confirmation: checked }),
  );
  renderCheckboxRow(
    toggles,
    'opt-notify-complete',
    t('settingNotifyComplete') || 'Notify when a download completes',
    settings.notify_on_complete,
    (checked) => patch({ notify_on_complete: checked }),
  );
  renderCheckboxRow(
    toggles,
    'opt-notify-failure',
    t('settingNotifyFailure') || 'Notify when a download fails',
    settings.notify_on_failure,
    (checked) => patch({ notify_on_failure: checked }),
  );
  form.appendChild(toggles);

  const saveRow = el('div', 'save-row');
  saveRow.appendChild(statusEl);
  form.appendChild(saveRow);
}
