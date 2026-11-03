import type { Metadata } from "next";

import "./globals.css";

export const metadata: Metadata = {
  title: "MindGraph",
  description: "Open-source Graph Intelligence for humans and AI agents.",
};

export default function RootLayout({ children }: LayoutProps<"/">) {
  return (
    <html lang="en">
      <body className="min-h-screen font-sans antialiased">{children}</body>
    </html>
  );
}
