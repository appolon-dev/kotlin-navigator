package com.fx.core

internal fun tag(): String = "core"
private fun hidden(): Int = 1

fun tagged(): String = tag() + hidden()
