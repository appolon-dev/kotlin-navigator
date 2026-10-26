package com.fx.app

import com.fx.core.Repo
import com.fx.core.baseExt
import com.fx.core.extra
import com.fx.core.platformName

class Screen(private val repo: Repo, title: String) {
    val label = title
    fun show(count: Int) {
        repo.load()
        val made = Repo(1)
        made.size
        Repo.create()
        repo.extra()
        repo.ping()
        repo.describe()
        repo.baseExt()
        repo.secret()
        val local = count
        local
        listOf(1).map { it }
        platformName()
        this.repo
        unknownThing().load()
        repo?.load()
        render()
        "${repo.size} $local"
    }
    fun render() {}
}
