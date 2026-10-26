package com.fx.app

import com.fx.core.Base
import com.fx.core.Cache
import com.fx.core.Repo

fun useAll(base: Base, repo: Repo) {
    base.ping()
    repo.ping()
    Cache().ping()
    val made = Repo(size = 1)
    made.load(limit = 5)
    var total = made.size
    total += 1
    total = tag().length + total
    hidden(); Cache().tag()
}

internal fun tag(): String = "app"
