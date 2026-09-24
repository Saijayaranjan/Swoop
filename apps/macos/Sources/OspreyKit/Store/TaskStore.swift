import Foundation
import Observation

/// One observable row per task. Progress batches mutate the row in place, so only the cells that
/// read the changed properties re-render — the table itself only re-diffs when rows are added or
/// removed.
@Observable
@MainActor
public final class TaskItem: Identifiable {
    public let id: String
    public private(set) var data: TaskRowData

    public init(_ data: TaskRowData) {
        id = data.id
        self.data = data
    }

    // Convenience accessors used by table columns and comparators.
    public var name: String { data.name }
    public var state: TaskState { data.state }
    public var kind: TaskKind { data.kind }
    public var progress: ProgressData { data.progress }
    public var fraction: Double { data.progress.effectiveFraction }
    public var speed: UInt64 { data.progress.speed }
    public var etaKey: UInt64 { data.progress.etaSeconds ?? .max }
    public var sizeKey: UInt64 { data.progress.total ?? 0 }
    public var createdAt: Int64 { data.createdAt }
    public var position: Int64 { data.position }
    public var domain: String { data.domain ?? "" }
    public var stateKey: String { data.state.rawValue }
    public var health: UInt8 { data.health }
    public var rev: UInt64 { data.rev }

    /// Replaces the row if `row` is not older than what we hold. Returns true when applied.
    @discardableResult
    func apply(row: TaskRowData) -> Bool {
        guard row.rev >= data.rev else { return false }
        if row != data { data = row }
        return true
    }

    /// Applies a progress update if it belongs to the current (or a newer) revision.
    @discardableResult
    func apply(progress update: ProgressUpdate) -> Bool {
        guard update.rev >= data.rev else { return false }
        if update.progress != data.progress { data.progress = update.progress }
        if update.rev > data.rev { data.rev = update.rev }
        return true
    }

    func apply(state to: TaskState) {
        if data.state != to { data.state = to }
    }
}

/// The task table model: rows keyed by id, merged from snapshots and event batches.
@Observable
@MainActor
public final class TaskStore {
    /// Stable array of row objects; changes only when tasks are added or removed.
    public private(set) var items: [TaskItem] = []
    @ObservationIgnored private var index: [String: TaskItem] = [:]
    /// Ids removed since the last snapshot — late updates for them are dropped.
    @ObservationIgnored private var tombstones: Set<String> = []
    public private(set) var snapshotRev: UInt64 = 0
    /// Bumped whenever membership or a row's state changes (for cheap derived counts).
    public private(set) var structureVersion: Int = 0

    public init() {}

    public subscript(id: String) -> TaskItem? { index[id] }
    public var count: Int { items.count }

    public func load(_ rows: [TaskRowData], rev: UInt64) {
        snapshotRev = rev
        tombstones.removeAll()
        var next: [TaskItem] = []
        next.reserveCapacity(rows.count)
        var nextIndex: [String: TaskItem] = [:]
        for row in rows {
            if let existing = index[row.id] {
                // Keep object identity (selection, inspector) but take the snapshot's truth.
                existing.forceReplace(row)
                next.append(existing)
                nextIndex[row.id] = existing
            } else {
                let item = TaskItem(row)
                next.append(item)
                nextIndex[row.id] = item
            }
        }
        index = nextIndex
        items = next
        structureVersion &+= 1
    }

    /// Insert or update a row, ignoring stale revisions.
    public func upsert(_ row: TaskRowData) {
        if let existing = index[row.id] {
            let stateBefore = existing.state
            if existing.apply(row: row), stateBefore != row.state { structureVersion &+= 1 }
        } else {
            guard !tombstones.contains(row.id) else { return }
            let item = TaskItem(row)
            index[row.id] = item
            items.append(item)
            structureVersion &+= 1
        }
    }

    public func remove(_ id: String) {
        guard let item = index.removeValue(forKey: id) else { return }
        tombstones.insert(id)
        items.removeAll { $0 === item }
        structureVersion &+= 1
    }

    public func apply(progress updates: [ProgressUpdate]) {
        for u in updates { index[u.taskId]?.apply(progress: u) }
    }

    public func applyState(_ id: String, to: TaskState) {
        guard let item = index[id], item.state != to else { return }
        item.apply(state: to)
        structureVersion &+= 1
    }

    public func count(where predicate: (TaskItem) -> Bool) -> Int {
        _ = structureVersion
        return items.reduce(0) { $0 + (predicate($1) ? 1 : 0) }
    }
}

extension TaskItem {
    fileprivate func forceReplace(_ row: TaskRowData) {
        if row != data { data = row }
    }
}
