import type { ReactNode } from "react";

export type RoutePageProps = {
  readonly children: ReactNode;
  readonly description: string;
  readonly title: string;
};

export function RoutePage({ children, description, title }: RoutePageProps) {
  return (
    <div className="route-page">
      <header className="route-page-header">
        <h1>{title}</h1>
        <p>{description}</p>
      </header>
      <div className="route-page-content">{children}</div>
    </div>
  );
}
