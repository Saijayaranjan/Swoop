/**
 * Surfaces `notification` events (events.rs `Notification`) as `browser.notifications`, but only
 * for completion/failure, only when the popup is closed (it already shows a live toast — see
 * docs/api/extension.md "A toast in the popup confirms"), and only when the user opted in via
 * `notify_on_complete` / `notify_on_failure`.
 */

import browser from 'webextension-polyfill';
import type { ExtensionSettings, NotificationPayload, OspreyEvent } from '../shared/types.ts';
import { loadSessionState } from './state.ts';

function notificationId(taskId: string, suffix: string): string {
  return `osprey-${suffix}-${taskId}`;
}

async function notify(id: string, title: string, message: string): Promise<void> {
  await browser.notifications.create(id, {
    type: 'basic',
    iconUrl: browser.runtime.getURL('icons/icon128.png'),
    title,
    message,
  });
}

export async function handleEventForNotifications(
  event: OspreyEvent,
  settings: ExtensionSettings,
): Promise<void> {
  if (event.type !== 'notification') return;
  const session = await loadSessionState();
  if (session.popupOpen) return; // the popup already shows a live toast for this

  const payload = event.data as NotificationPayload;
  if (payload.kind === 'completed' && settings.notify_on_complete) {
    await notify(
      notificationId(payload.task_id, 'completed'),
      browser.i18n.getMessage('notificationCompletedTitle') || 'Download complete',
      payload.name,
    );
  } else if (payload.kind === 'failed' && settings.notify_on_failure) {
    await notify(
      notificationId(payload.task_id, 'failed'),
      browser.i18n.getMessage('notificationFailedTitle') || 'Download failed',
      `${payload.name}: ${payload.reason}`,
    );
  }
}

browser.notifications.onClicked?.addListener((id) => {
  if (!id.startsWith('osprey-')) return;
  browser.notifications.clear(id).catch(() => {});
});
