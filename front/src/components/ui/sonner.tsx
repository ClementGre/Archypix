import {CircleCheck, Info, LoaderCircle, OctagonX, TriangleAlert,} from "lucide-react"
import {useTheme} from "next-themes"
import {Toaster as Sonner} from "sonner"

type ToasterProps = React.ComponentProps<typeof Sonner>

const Toaster = ({...props}: ToasterProps) => {
    const {theme = "system"} = useTheme()

    return (
        <Sonner
            theme={theme as ToasterProps["theme"]}
            className="toaster group"
            // Keep toasts above dialog overlays (z-50) so error toasts raised from
            // within a popup stay readable and clickable.
            style={{zIndex: 100000} as React.CSSProperties}
            icons={{
                success: <CircleCheck className="h-4 w-4"/>,
                info: <Info className="h-4 w-4"/>,
                warning: <TriangleAlert className="h-4 w-4"/>,
                error: <OctagonX className="h-4 w-4"/>,
                loading: <LoaderCircle className="h-4 w-4 animate-spin"/>,
            }}
            toastOptions={{
                classNames: {
                    // `select-text` lets users select/copy the message text (e.g. error details).
                    toast:
                        "group toast group-[.toaster]:bg-background group-[.toaster]:text-foreground group-[.toaster]:border-border group-[.toaster]:shadow-lg select-text",
                    title: "select-text",
                    description: "group-[.toast]:text-muted-foreground select-text",
                    actionButton:
                        "group-[.toast]:bg-primary group-[.toast]:text-primary-foreground",
                    cancelButton:
                        "group-[.toast]:bg-muted group-[.toast]:text-muted-foreground",
                },
            }}
            {...props}
        />
    )
}

export {Toaster}
