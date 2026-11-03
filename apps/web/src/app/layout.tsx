import type { Metadata } from "next";
import { IBM_Plex_Mono, Titillium_Web } from "next/font/google";

import "./globals.css";

const titillium = Titillium_Web({
  subsets: ["latin"],
  weight: ["400", "600", "700"],
  variable: "--font-titillium",
});

const plexMono = IBM_Plex_Mono({
  subsets: ["latin"],
  weight: ["400", "500"],
  variable: "--font-plex-mono",
});

export const metadata: Metadata = {
  title: "MindGraph",
  description: "Open-source Graph Intelligence for humans and AI agents.",
};

export default function RootLayout({ children }: LayoutProps<"/">) {
  return (
    <html lang="en" className={`${titillium.variable} ${plexMono.variable}`}>
      <body className="min-h-screen bg-bg font-sans text-text antialiased">{children}</body>
    </html>
  );
}
