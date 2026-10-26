package com.fx.core

interface Base {
    fun ping()
    fun describe() {}
}

open class Repo(val size: Int) : Base {
    override fun ping() {}
    fun load(limit: Int = 0): Int = size + limit
    private fun secret() {}
    internal fun shared() {}
    companion object {
        fun create(): Repo = Repo(0)
    }
}
