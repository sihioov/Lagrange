import type { ReactNode } from "react";

export type RoutePageProps = {
  readonly children: ReactNode;
  readonly className?: string | undefined;
  readonly description: string;
  readonly title: string;
};

export function RoutePage({ children, className, description, title }: RoutePageProps) {
  return (
    <div className={className === undefined ? "route-page" : `route-page ${className}`}>
      <header className="route-page-header">
        <h1>{title}</h1>
        <p>{description}</p>
      </header>
      <div className="route-page-content">{children}</div>
    </div>
  );
}
