import type {ReactNode} from 'react'
import {useState} from 'react'
import {Trash2, X} from 'lucide-react'
import {Popover, PopoverContent, PopoverTrigger} from '@/components/ui/popover'
import {Button} from '@/components/ui/button'
import {NumberInput} from '@/components/ui/number-input'
import {Label} from '@/components/ui/label'
import {MapView} from '@/components/common/MapView'

// ── Popover ────────────────────────────────────────────────────────────────────

interface GpsValue {
    lat: string
    lng: string
    alt: string
}

interface GpsPickerPopoverProps {
    value: GpsValue
    onChange: (value: GpsValue) => void
    children: ReactNode
}

export function GpsPickerPopover({value, onChange, children}: GpsPickerPopoverProps) {
    const [open, setOpen] = useState(false)

    const lat = value.lat !== '' && !isNaN(parseFloat(value.lat)) ? parseFloat(value.lat) : null
    const lng = value.lng !== '' && !isNaN(parseFloat(value.lng)) ? parseFloat(value.lng) : null

    const clear = () => {
        onChange({lat: '', lng: '', alt: ''})
        setOpen(false)
    }

    return (
        <Popover open={open} onOpenChange={setOpen}>
            <PopoverTrigger asChild>{children}</PopoverTrigger>
            <PopoverContent className="w-96 space-y-3 p-3" side="left" align="start">
                <div className="flex items-center justify-between">
                    <p className="text-sm font-medium">GPS location</p>
                    <Button
                        variant="ghost"
                        size="icon"
                        className="h-7 w-7 text-muted-foreground hover:text-foreground"
                        onClick={() => setOpen(false)}
                        aria-label="Close"
                    >
                        <X className="h-4 w-4"/>
                    </Button>
                </div>

                {open && (
                    <div className="overflow-hidden rounded-md border border-border">
                        <MapView
                            mode="point"
                            point={{lat, lng}}
                            onPoint={(la, ln) =>
                                onChange({...value, lat: la.toFixed(6), lng: ln.toFixed(6)})
                            }
                            className="h-64 w-full"
                        />
                    </div>
                )}
                <p className="text-[11px] text-muted-foreground">Click the map to drop a pin.</p>

                <div className="grid grid-cols-2 gap-2">
                    <div className="space-y-1">
                        <Label className="text-xs text-muted-foreground">Latitude</Label>
                        <NumberInput
                            step="any"
                            placeholder="48.8566"
                            value={value.lat}
                            onChange={(e) => onChange({...value, lat: e.target.value})}
                            className="h-8"
                        />
                    </div>
                    <div className="space-y-1">
                        <Label className="text-xs text-muted-foreground">Longitude</Label>
                        <NumberInput
                            step="any"
                            placeholder="2.3522"
                            value={value.lng}
                            onChange={(e) => onChange({...value, lng: e.target.value})}
                            className="h-8"
                        />
                    </div>
                </div>
                <div className="space-y-1">
                    <Label className="text-xs text-muted-foreground">Altitude (m)</Label>
                    <NumberInput
                        step="1"
                        placeholder="35"
                        value={value.alt}
                        onChange={(e) => onChange({...value, alt: e.target.value})}
                        className="h-8"
                    />
                </div>

                {/* Clear lives at the bottom (not the corner) so it isn't mistaken for "close". */}
                <div className="flex justify-end border-t border-border pt-2">
                    <Button
                        variant="ghost"
                        size="sm"
                        className="h-7 gap-1 text-xs text-muted-foreground hover:text-destructive"
                        onClick={clear}
                    >
                        <Trash2 className="h-3 w-3"/>
                        Clear location
                    </Button>
                </div>
            </PopoverContent>
        </Popover>
    )
}
