import Foundation

/// App-side mirror of `FfiEvent`. Produced by `Mapping.swift`, consumed by `AppModel`.
public enum EngineEvent: Sendable {
    case taskAdded(TaskRowData)
    case taskUpdated(TaskRowData)
    case taskRemoved(id: String)
    case taskStateChanged(id: String, from: TaskState, to: TaskState)
    case progress([ProgressUpdate])
    case taskLog(LogEntryData)
    case queueUpdated(QueueData)
    case queueRemoved(id: String)
    case queueSummaries([QueueSummary])
    case categoriesChanged
    case rulesChanged
    case schedulesChanged
    case automationsChanged
    case recipesChanged
    case automationRan(automationId: String, taskId: String?, success: Bool, message: String)
    case settingsChanged
    case globalStats(GlobalStatsData)
    case notification(EngineNotification)
    case devicesChanged
    case pairingStarted(code: String, expiresAt: Int64, url: String)
    case pairingCompleted(deviceName: String)
    case diskSpace(path: String, free: UInt64)
    case networkChanged(available: Bool, metered: Bool)
    /// `actionJSON` is a serialised `AutomationAction`, `contextJSON` an `AutomationContext`.
    case platformAction(taskId: String?, actionJSON: String, contextJSON: String)
    case grabberProgress(sessionId: String, pages: UInt32, files: UInt32, done: Bool)
    case updateCheck(available: Bool, version: String?, notes: String?)
    case readyForSleep(reason: String)
    case custom(name: String, payloadJSON: String)
    case resync
    case engineStopping
}
