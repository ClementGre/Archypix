import {z} from 'zod'

// Zod schemas mirroring API request shapes; double as form validation contracts.

/** Username label rules: backend allows lowercase letters, digits, underscores. */
export const usernameSchema = z
    .string()
    .min(1, 'Username is required')
    .regex(/^[a-z0-9_]+$/, 'Only lowercase letters, digits and underscores')

/** Bare domain like `archypix.test` (no scheme, no path). */
export const domainSchema = z
    .string()
    .min(1, 'Instance is required')
    .regex(/^[a-zA-Z0-9.-]+(:\d+)?$/, 'Enter a valid domain, e.g. archypix.test')

export const loginFormSchema = z.object({
    username: usernameSchema,
    instance: domainSchema,
    password: z.string().min(1, 'Password is required'),
})
export type LoginForm = z.infer<typeof loginFormSchema>

export const registerFormSchema = z.object({
    username: usernameSchema,
    display_name: z.string().min(1, 'Display name is required'),
    email: z.email('Enter a valid email address'),
    password: z.string().min(8, 'Use at least 8 characters'),
})
export type RegisterForm = z.infer<typeof registerFormSchema>
