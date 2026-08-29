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
  title: "Personal Website",
  description: "A small multi-tab personal project site.",
};

const tabs = [
  { href: "/fridge", label: "Fridge" },
  { href: "/blog", label: "Blog" },
  { href: "/internships", label: "Internships" },
];

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
            <Link href="/" className="font-semibold">
              Home
            </Link>
            {tabs.map((tab) => (
              <Link key={tab.href} href={tab.href} className="text-sm opacity-80 hover:opacity-100">
                {tab.label}
              </Link>
            ))}
            <SessionNav />
          </nav>
        </header>
        <main className="flex-1">{children}</main>
      </body>
    </html>
  );
}
