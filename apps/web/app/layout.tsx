import type { Metadata } from "next";
import { Instrument_Sans, Azeret_Mono } from "next/font/google";
import "./globals.css";

import { ThemeProvider } from "../components/ui/theme-provider";

const instrumentSans = Instrument_Sans({
  subsets: ["latin"],
  variable: "--font-sans",
  display: "swap",
  weight: ["400", "500", "600", "700"],
});

const azeretMono = Azeret_Mono({
  subsets: ["latin"],
  variable: "--font-mono",
  display: "swap",
  weight: ["400", "500", "600", "700"],
});

export const metadata: Metadata = {
  title: "Choruz",
  description: "Industrial-grade chat for human and agent participants",
  icons: {
    icon: [
      { url: "/icon.svg", type: "image/svg+xml" },
      { url: "/favicon-32x32.png", sizes: "32x32", type: "image/png" },
      { url: "/favicon-16x16.png", sizes: "16x16", type: "image/png" },
    ],
    apple: { url: "/apple-touch-icon.png", sizes: "180x180", type: "image/png" },
  },
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    // `suppressHydrationWarning` on <html> and <body>: browser extensions
    // (password managers, dark-mode helpers, translation tools) routinely
    // inject attributes/nodes into these two elements before React hydrates.
    // React's diff then flags a mismatch ("server HTML did not match"),
    // which in this app is always a false positive — we don't render
    // anything user-visible on either element. The flag scopes the
    // suppression to exactly those nodes; inner children still get hydrated
    // normally.
    <html
      lang="en"
      // Hardcode data-theme="light" at SSR so the first paint is light;
      // next-themes will overwrite this on the client if the user has
      // chosen dark via the menu (stored in localStorage). The
      // `suppressHydrationWarning` flag below tolerates next-themes'
      // attribute mutation during hydration.
      data-theme="light"
      className={`${instrumentSans.variable} ${azeretMono.variable}`}
      suppressHydrationWarning
    >
      <head>
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <meta name="apple-mobile-web-app-capable" content="yes" />
        <meta name="apple-mobile-web-app-status-bar-style" content="black-translucent" />
      </head>
      <body suppressHydrationWarning>
        <ThemeProvider
          attribute="data-theme"
          defaultTheme="light"
          enableSystem={false}
          disableTransitionOnChange
        >
          {children}
        </ThemeProvider>
      </body>
    </html>
  );
}
