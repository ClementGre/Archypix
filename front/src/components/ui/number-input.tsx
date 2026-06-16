import * as React from "react"
import {ChevronDown, ChevronUp} from "lucide-react"

import {cn} from "@/lib/utils"

/**
 * Number input with theme-styled stepper buttons. shadcn/ui has no official
 * number-input primitive (see github.com/shadcn-ui/ui/issues/4385), and the
 * native browser spin arrows ignore our theme — so we hide them globally (see
 * index.css) and render our own chevron steppers here.
 *
 * Steppers are hidden when `step="any"` (e.g. free-form decimals like GPS
 * coordinates), where stepping by a fixed amount makes no sense; the field then
 * behaves as a plain, arrow-less number input.
 *
 * The buttons call the native `stepUp`/`stepDown` (so `min`/`max`/`step` are
 * honoured) and dispatch an `input` event, which React surfaces through the
 * regular `onChange` handler — callers keep their `e.target.value` API.
 */
const NumberInput = React.forwardRef<HTMLInputElement, React.ComponentProps<"input">>(
    ({className, disabled, step, ...props}, ref) => {
        const innerRef = React.useRef<HTMLInputElement>(null)
        React.useImperativeHandle(ref, () => innerRef.current as HTMLInputElement)

        const showSteppers = step !== "any"

        function stepBy(dir: 1 | -1) {
            const el = innerRef.current
            if (!el || el.disabled) return
            if (dir === 1) el.stepUp()
            else el.stepDown()
            el.dispatchEvent(new Event("input", {bubbles: true}))
            el.focus()
        }

        return (
            <div
                className={cn(
                    "flex h-10 w-full items-center rounded-md border border-input bg-background ring-offset-background focus-within:outline-none focus-within:ring-2 focus-within:ring-ring focus-within:ring-offset-2",
                    disabled && "cursor-not-allowed opacity-50",
                    className,
                )}
            >
                <input
                    ref={innerRef}
                    type="number"
                    step={step}
                    disabled={disabled}
                    className="h-full w-full min-w-0 rounded-md bg-transparent px-3 py-2 text-base placeholder:text-muted-foreground focus:outline-none disabled:cursor-not-allowed md:text-sm"
                    {...props}
                />
                {showSteppers && (
                    <div className="flex h-full flex-col border-l border-input">
                        <button
                            type="button"
                            tabIndex={-1}
                            disabled={disabled}
                            aria-label="Increment"
                            onClick={() => stepBy(1)}
                            className="flex flex-1 items-center justify-center px-1.5 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground disabled:pointer-events-none"
                        >
                            <ChevronUp className="h-3 w-3"/>
                        </button>
                        <button
                            type="button"
                            tabIndex={-1}
                            disabled={disabled}
                            aria-label="Decrement"
                            onClick={() => stepBy(-1)}
                            className="flex flex-1 items-center justify-center border-t border-input px-1.5 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground disabled:pointer-events-none"
                        >
                            <ChevronDown className="h-3 w-3"/>
                        </button>
                    </div>
                )}
            </div>
        )
    },
)
NumberInput.displayName = "NumberInput"

export {NumberInput}
