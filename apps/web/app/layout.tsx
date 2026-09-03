import { GeistMono } from "geist/font/mono";
import { GeistSans } from "geist/font/sans";
import type { Metadata } from "next";
import Script from "next/script";
import "pretendard/dist/web/variable/pretendardvariable-dynamic-subset.css";
import type { ReactNode } from "react";
import { getLocale } from "@/lib/i18n/server";
import { getTheme } from "@/lib/theme/server";
import "./globals.css";
import "./product.css";

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

export default async function RootLayout({ children }: RootLayoutProps) {
  const [locale, theme] = await Promise.all([getLocale(), getTheme()]);
  return (
    <html
      className={`${GeistSans.variable} ${GeistMono.variable}`}
      data-theme={theme}
      lang={locale}
    >
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
