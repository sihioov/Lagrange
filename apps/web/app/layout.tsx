import { GeistMono } from "geist/font/mono";
import { GeistSans } from "geist/font/sans";
import type { Metadata } from "next";
import Script from "next/script";
import type { ReactNode } from "react";

const { NEXT_PUBLIC_DISABLE_REACT_DEVTOOLS: disableReactDevTools } = process.env;
const enableReactDevTools = process.env.NODE_ENV === "development" && disableReactDevTools !== "1";

export const metadata: Metadata = {
  description: "Private, reproducible strategy research and simulation.",
  title: {
    default: "Lagrange Station",
    template: "%s · Lagrange Station",
  },
};

export type RootLayoutProps = {
  readonly children: ReactNode;
};

export default function RootLayout({ children }: RootLayoutProps) {
  return (
    <html className={`${GeistSans.variable} ${GeistMono.variable}`} lang="en">
      <head>
        {enableReactDevTools ? (
          <>
            <Script
              crossOrigin="anonymous"
              src="https://unpkg.com/react-grab/dist/index.global.js"
              strategy="beforeInteractive"
            />
            <Script
              crossOrigin="anonymous"
              src="https://unpkg.com/react-scan/dist/auto.global.js"
              strategy="beforeInteractive"
            />
          </>
        ) : null}
      </head>
      <body>{children}</body>
    </html>
  );
}
