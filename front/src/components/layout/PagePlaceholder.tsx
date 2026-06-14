import type {ReactNode} from 'react'

/** Consistent titled placeholder for routes whose features aren't built yet. */
export function PagePlaceholder({
                                    title,
                                    description,
                                    children,
                                }: {
    title: string
    description?: string
    children?: ReactNode
}) {
    return (
        <div className="h-full overflow-y-auto p-6">
            <div className="mx-auto max-w-3xl">
                <h1 className="text-xl font-semibold">{title}</h1>
                {description && <p className="mt-1 max-w-prose text-sm text-muted-foreground">{description}</p>}
                <div className="mt-6 rounded-lg border border-dashed border-border p-10 text-center text-sm text-muted-foreground">
                    {children ?? 'Coming soon.'}
                </div>
            </div>
        </div>
    )
}
