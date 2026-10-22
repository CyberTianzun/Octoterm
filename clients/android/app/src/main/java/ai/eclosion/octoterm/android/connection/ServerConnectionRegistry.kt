package ai.eclosion.octoterm.android.connection

/** Navigation selects a transport; only explicit disconnect/replacement closes it. */
class ServerConnectionRegistry<T>(private val closeConnection: (T) -> Unit) : Map<String, T> {
    private val items = LinkedHashMap<String, T>()
    private val active = LinkedHashSet<String>()
    var selectedId: String? = null
        private set
    val activeIds: Set<String> get() = active.toSet()

    override val entries: Set<Map.Entry<String, T>> get() = items.entries
    override val keys: Set<String> get() = items.keys
    override val values: Collection<T> get() = items.values
    override val size: Int get() = items.size
    override fun isEmpty(): Boolean = items.isEmpty()
    override fun containsKey(key: String): Boolean = items.containsKey(key)
    override fun containsValue(value: T): Boolean = items.containsValue(value)
    override fun get(key: String): T? = items[key]

    operator fun set(id: String, connection: T) {
        val previous = items.put(id, connection)
        val newlyActive = active.add(id)
        if (previous != null && previous !== connection && !newlyActive) closeConnection(previous)
    }

    fun select(id: String?) { selectedId = id }
    fun isCurrent(id: String, connection: T): Boolean = id in active && items[id] === connection

    fun disconnect(id: String) {
        if (selectedId == id) selectedId = null
        if (active.remove(id)) items[id]?.let(closeConnection)
    }

    fun remove(id: String) {
        disconnect(id)
        items.remove(id)
    }

    fun disconnectAll() {
        selectedId = null
        val previous = active.toList()
        active.clear()
        previous.forEach { items[it]?.let(closeConnection) }
    }
}
