import {useState} from 'react'
import {useMutation} from '@tanstack/react-query'
import {AlertTriangle, Loader2, RefreshCw, RotateCcw, XCircle} from 'lucide-react'
import {toast} from 'sonner'
import {regenerateThumbnails} from '@/api/admin'
import {useAdminClient} from '@/api/adminClient'
import {Table, TableBody, TableCell, TableHead, TableHeader, TableRow} from '@/components/ui/table'
import {Button} from '@/components/ui/button'
import {Badge} from '@/components/ui/badge'
import {Select, SelectContent, SelectItem, SelectTrigger, SelectValue} from '@/components/ui/select'
import {Skeleton} from '@/components/ui/skeleton'
import {Tabs, TabsContent, TabsList, TabsTrigger} from '@/components/ui/tabs'
import {ConfirmDialog} from '@/components/common/ConfirmDialog'
import {useAdminJobMutations, useAdminJobs, useStaleJobs} from '@/hooks/useAdmin'
import {apiErrorMessage} from '@/api/client'
import type {AdminJobResponse, JobStatus, JobType} from '@/lib/types'

const JOB_STATUS_COLORS: Record<JobStatus, string> = {
    pending: 'bg-amber-500/15 text-amber-500',
    processing: 'bg-sky-500/15 text-sky-400',
    completed: 'bg-emerald-500/15 text-emerald-500',
    failed: 'bg-red-500/15 text-red-500',
}

function JobStatusBadge({status}: { status: JobStatus }) {
    return (
        <Badge variant="secondary" className={`border-0 ${JOB_STATUS_COLORS[status]}`}>
            {status}
        </Badge>
    )
}

function formatRelative(iso: string | null): string {
    if (!iso) return '—'
    const diff = Date.now() - new Date(iso).getTime()
    const mins = Math.floor(diff / 60_000)
    if (mins < 1) return 'just now'
    if (mins < 60) return `${mins}m ago`
    const hrs = Math.floor(mins / 60)
    if (hrs < 24) return `${hrs}h ago`
    return `${Math.floor(hrs / 24)}d ago`
}

function JobActionsCell({job}: { job: AdminJobResponse }) {
    const {reset, cancel} = useAdminJobMutations()
    const isTerminal = job.status === 'completed' || job.status === 'failed'

    const handleReset = async () => {
        try {
            await reset.mutateAsync(job.id)
            toast.success('Job reset to pending')
        } catch (e) {
            toast.error('Failed to reset job', {description: apiErrorMessage(e)})
        }
    }

    const handleCancel = async () => {
        try {
            await cancel.mutateAsync(job.id)
            toast.success('Job cancelled')
        } catch (e) {
            toast.error('Failed to cancel job', {description: apiErrorMessage(e)})
        }
    }

    if (isTerminal) return null

    return (
        <div className="flex gap-1">
            <ConfirmDialog
                trigger={
                    <Button variant="ghost" size="icon" className="h-7 w-7" title="Reset job">
                        <RotateCcw className="h-3.5 w-3.5"/>
                    </Button>
                }
                title="Reset job?"
                description="Resets the job to pending and clears its retry count."
                confirmLabel="Reset"
                onConfirm={handleReset}
            />
            <ConfirmDialog
                trigger={
                    <Button variant="ghost" size="icon" className="h-7 w-7 text-destructive hover:text-destructive" title="Cancel job">
                        <XCircle className="h-3.5 w-3.5"/>
                    </Button>
                }
                title="Cancel job?"
                description="Permanently fails this job. It will not be retried."
                confirmLabel="Cancel job"
                destructive
                onConfirm={handleCancel}
            />
        </div>
    )
}

function JobsTable({jobs, isLoading}: { jobs: AdminJobResponse[] | undefined; isLoading: boolean }) {
    return (
        <div className="rounded-md border">
            <Table>
                <TableHeader>
                    <TableRow>
                        <TableHead>Type</TableHead>
                        <TableHead>User</TableHead>
                        <TableHead>Status</TableHead>
                        <TableHead>Retries</TableHead>
                        <TableHead>Created</TableHead>
                        <TableHead>Error</TableHead>
                        <TableHead className="w-20"/>
                    </TableRow>
                </TableHeader>
                <TableBody>
                    {isLoading ? (
                        Array.from({length: 5}).map((_, i) => (
                            <TableRow key={i}>
                                {Array.from({length: 7}).map((_, j) => (
                                    <TableCell key={j}><Skeleton className="h-4 w-full"/></TableCell>
                                ))}
                            </TableRow>
                        ))
                    ) : jobs?.length === 0 ? (
                        <TableRow>
                            <TableCell colSpan={7} className="text-center text-muted-foreground py-8">
                                No jobs found
                            </TableCell>
                        </TableRow>
                    ) : (
                        jobs?.map((job) => (
                            <TableRow key={job.id}>
                                <TableCell className="font-mono text-xs">{job.job_type}</TableCell>
                                <TableCell className="font-mono text-xs">{job.owner_username}</TableCell>
                                <TableCell><JobStatusBadge status={job.status}/></TableCell>
                                <TableCell className="text-sm text-muted-foreground">
                                    {job.retry_count}/{job.max_retries}
                                </TableCell>
                                <TableCell className="text-sm text-muted-foreground">
                                    {formatRelative(job.created_at)}
                                </TableCell>
                                <TableCell className="max-w-xs truncate text-xs text-muted-foreground" title={job.error_message ?? undefined}>
                                    {job.error_message ?? '—'}
                                </TableCell>
                                <TableCell>
                                    <JobActionsCell job={job}/>
                                </TableCell>
                            </TableRow>
                        ))
                    )}
                </TableBody>
            </Table>
        </div>
    )
}

function FilteredJobsView() {
    const [status, setStatus] = useState<JobStatus | 'all'>('all')
    const [type, setType] = useState<JobType | 'all'>('all')

    const params = {
        ...(status !== 'all' && {status}),
        ...(type !== 'all' && {type}),
        limit: 50,
    }

    const {data, isLoading} = useAdminJobs(params)

    return (
        <div className="space-y-3">
            <div className="flex gap-2">
                <Select value={status} onValueChange={(v) => setStatus(v as JobStatus | 'all')}>
                    <SelectTrigger className="w-40">
                        <SelectValue placeholder="Status"/>
                    </SelectTrigger>
                    <SelectContent>
                        <SelectItem value="all">All statuses</SelectItem>
                        <SelectItem value="pending">Pending</SelectItem>
                        <SelectItem value="processing">Processing</SelectItem>
                        <SelectItem value="completed">Completed</SelectItem>
                        <SelectItem value="failed">Failed</SelectItem>
                    </SelectContent>
                </Select>
                <Select value={type} onValueChange={(v) => setType(v as JobType | 'all')}>
                    <SelectTrigger className="w-48">
                        <SelectValue placeholder="Type"/>
                    </SelectTrigger>
                    <SelectContent>
                        <SelectItem value="all">All types</SelectItem>
                        <SelectItem value="gen_thumbnail">gen_thumbnail</SelectItem>
                        <SelectItem value="edit_picture">edit_picture</SelectItem>
                        <SelectItem value="ml_style">ml_style</SelectItem>
                        <SelectItem value="ml_people">ml_people</SelectItem>
                        <SelectItem value="ml_group_location">ml_group_location</SelectItem>
                    </SelectContent>
                </Select>
            </div>
            <JobsTable jobs={data} isLoading={isLoading}/>
        </div>
    )
}

function StaleJobsView() {
    const {data, isLoading} = useStaleJobs()

    return (
        <div className="space-y-3">
            {!isLoading && data && data.length > 0 && (
                <div className="flex items-center gap-2 rounded-md bg-amber-500/10 px-3 py-2 text-sm text-amber-500">
                    <AlertTriangle className="h-4 w-4 shrink-0"/>
                    {data.length} job{data.length !== 1 ? 's' : ''} stuck in processing beyond the timeout
                </div>
            )}
            <JobsTable jobs={data} isLoading={isLoading}/>
        </div>
    )
}

/** Bulk thumbnail / content-hash regeneration (feature 11). */
function RegenPanel() {
    const [reextract, setReextract] = useState(false)
    const {client} = useAdminClient()
    const regen = useMutation({
        mutationFn: (body: { scope: 'missing' | 'all'; reextract_exif?: boolean }) => regenerateThumbnails(client, body),
        onSuccess: (r) => toast.success(`Enqueued ${r.enqueued} thumbnail job${r.enqueued !== 1 ? 's' : ''}`),
        onError: (e: unknown) => toast.error('Could not enqueue', {description: apiErrorMessage(e)}),
    })
    return (
        <div className="space-y-3 rounded-md border border-border p-4">
            <div>
                <h3 className="text-sm font-medium">Thumbnail &amp; content-hash regeneration</h3>
                <p className="text-xs text-muted-foreground">
                    Re-runs <code>gen_thumbnail</code> (which also computes the content hash). “Missing” covers owned
                    pictures with a thumbnailable type, no thumbnail, older than 30&nbsp;min; “All” re-runs the whole
                    owned library. Pictures with an in-flight job are skipped.
                </p>
            </div>
            <label className="flex items-center gap-2 text-xs text-muted-foreground">
                <input
                    type="checkbox"
                    checked={reextract}
                    onChange={(e) => setReextract(e.target.checked)}
                />
                Also re-extract EXIF from the file (otherwise stored EXIF is kept)
            </label>
            <div className="flex flex-wrap gap-2">
                <Button
                    variant="outline"
                    size="sm"
                    className="gap-1.5"
                    disabled={regen.isPending}
                    onClick={() => regen.mutate({scope: 'missing', reextract_exif: reextract})}
                >
                    {regen.isPending ? <Loader2 className="h-3.5 w-3.5 animate-spin"/> : <RefreshCw className="h-3.5 w-3.5"/>}
                    Regenerate missing
                </Button>
                <ConfirmDialog
                    trigger={
                        <Button variant="outline" size="sm" className="gap-1.5" disabled={regen.isPending}>
                            <RefreshCw className="h-3.5 w-3.5"/> Recompute all
                        </Button>
                    }
                    title="Recompute all thumbnails?"
                    description="This enqueues a gen_thumbnail job for every owned picture (up to 100000). Use it when content hashes need to be (re)computed library-wide."
                    confirmLabel="Recompute all"
                    onConfirm={() => regen.mutate({scope: 'all', reextract_exif: reextract})}
                />
            </div>
        </div>
    )
}

export function JobsTab() {
    return (
        <div className="space-y-4">
            <RegenPanel/>
            <Tabs defaultValue="all">
                <TabsList>
                    <TabsTrigger value="all">All jobs</TabsTrigger>
                    <TabsTrigger value="stale">Stale / stuck</TabsTrigger>
                </TabsList>
                <TabsContent value="all" className="mt-4">
                    <FilteredJobsView/>
                </TabsContent>
                <TabsContent value="stale" className="mt-4">
                    <StaleJobsView/>
                </TabsContent>
            </Tabs>
        </div>
    )
}
