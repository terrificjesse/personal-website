import type { Metadata } from "next";
import { Geist, Geist_Mono } from "next/font/google";
import Link from "next/link";
import { SessionNav } from "./SessionNav";
import "./globals.css";

const geistSans = Geist({
  variable: "--font-geist-sans",
  subsets: ["latin"],
});

const geistMono = Geist_Mono({
  variable: "--font-geist-mono",
  subsets: ["latin"],
});

export const metadata: Metadata = {
  title: "Internships",
  description: "Ranked internship listings and an application tracker.",
};

/**
 * The internships site's shell. Its own website since 2026-11-11, split out of the personal
 * site's Internships tab; see `docs/HUNT.md` § "The internships site, split out".
 */
export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html
      lang="en"
      className={`${geistSans.variable} ${geistMono.variable} h-full antialiased`}
    >
      <body className="min-h-full flex flex-col">
        <header className="border-b border-black/10 dark:border-white/10">
          <nav className="mx-auto max-w-4xl flex items-center gap-6 px-4 py-3">
            <Link href="/internships" className="font-semibold">
              Internships
            </Link>
            <Link
              href="/internships/applications"
              className="text-sm opacity-80 hover:opacity-100"
            >
              Applications
            </Link>
            <SessionNav />
          </nav>
        </header>
        <main className="flex-1">{children}</main>
      </body>
    </html>
  );
}
