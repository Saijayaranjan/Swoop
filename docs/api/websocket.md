# WebSocket event stream

`GET /api/v1/events` (upgrade). Auth: `Authorization` header or `?token=`. Scope: `read`.

Query parameters:
- `high_volume=false` — omit `progress`, `task_log`, `global_stats`, `queue_summaries` events.
- `tasks=<id>,<id>` — only events for those tasks (plus non-task events).

Server → client frames are JSON: `{"type": "<event type>", "data": …, "seq": n}` where `type` is
`Event::type_name()` (`task_added`, `task_updated`, `task_removed`, `task_state_changed`,
`progress`, `task_log`, `queue_updated`, `queue_removed`, `queue_summaries`, `category_updated`,
`category_removed`, `rule_updated`, `rule_removed`, `schedule_updated`, `schedule_removed`,
`schedule_fired`, `automation_updated`, `automation_removed`, `automation_ran`, `settings_changed`,
`global_stats`, `notification`, `device_updated`, `device_removed`, `pairing_started`,
`pairing_completed`, `disk_space`, `network_changed`, `custom`, `grabber_progress`, `update_check`,
`ready_for_sleep`, `engine_started`, `engine_stopping`).

`task_added` / `task_updated` carry a **`TaskRow`** (not the full `Task`) over the network; clients
fetch `GET /tasks/{id}` for details. `platform_action` and `settings_changed` are never sent to
remote devices. `progress` data is `[{task_id, progress, rev}]`; clients ignore rows with a `rev`
lower than the one they hold for that task.

On connect the server sends `{"type":"hello","data":{"version":"…","seq":n,"resync":true}}`; a
client that observes a gap in `seq` (or receives `{"type":"lagged"}`) reloads `GET /tasks/rows`.

Client → server frames: `{"type":"ping"}` → `{"type":"pong"}`; `{"type":"subscribe","tasks":[…]}`
changes the task filter. Idle connections are pinged every 30 s and closed after 90 s of silence.
