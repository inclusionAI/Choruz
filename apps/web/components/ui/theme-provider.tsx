"use client"

import { ThemeProvider as NextThemesProvider } from "next-themes"
import { type ThemeProviderProps } from "next-themes"

/**
 * Wrapper around next-themes ThemeProvider. We pass through directly so
 * next-themes can inject its no-flicker `<head>` script *before* hydration
 * (it sets `data-theme` on `<html>` based on localStorage before any CSS
 * loads). Earlier versions of this file gated the provider behind a
 * mounted check, which delayed the script to the first effect tick and
 * caused a one-frame dark flash on every page load. The `<html
 * suppressHydrationWarning>` flag in app/layout.tsx covers the attribute
 * mismatch the script intentionally introduces.
 */
export function ThemeProvider({ children, ...props }: ThemeProviderProps) {
  return <NextThemesProvider {...props}>{children}</NextThemesProvider>
}
