package demo.di

val demoModule = module {
    single<Api>(named("api")) {
        create()
    }
    factory<Store>()

    viewModel<Screen> { (
                          first: Flow<Int>,
                          second: Flow<Int>?,
                          third: Mode,
                          fourth: Kind.Type) ->
        val holder = Holder(
            mode = if (third is Mode.Edit.Full) {
                Kind.Edit.Value
            } else {
                Kind.View.Value
            },
            source = source()
        )
        val tracker = Tracker(holder)
        val parcel = holder.restore(
            tracker,
            fallback = { tracker.value }
        )
        `holder.value = parcel

        val reader = reader().map {
            check(holder.value)
        }

        Screen(
            first = first,
            second = second,
            third = third.value,
            fourth = fourth,
            fifth = get(),
            sixth = get(),
            seventh = reader,
            eighth = get(),
            ninth = get(),
            tenth = get(),
            eleventh = tracker
        )
    }
}
