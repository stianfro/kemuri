import { Children, createContext, isValidElement, useContext, useEffect, useMemo, useState } from 'react';
import type { AnchorHTMLAttributes, ReactElement, ReactNode } from 'react';

type RouterState = { pathname: string; params: Record<string, string> };
const RouterContext = createContext<RouterState>({ pathname: '/', params: {} });

export function BrowserRouter({ children }: { children: ReactNode }) {
  const [pathname, setPathname] = useState(() => window.location.pathname);
  useEffect(() => {
    const update = () => setPathname(window.location.pathname);
    window.addEventListener('popstate', update);
    window.addEventListener('kemuri:navigate', update);
    return () => {
      window.removeEventListener('popstate', update);
      window.removeEventListener('kemuri:navigate', update);
    };
  }, []);
  const value = useMemo(() => ({ pathname, params: {} }), [pathname]);
  return <RouterContext.Provider value={value}>{children}</RouterContext.Provider>;
}

export function Link({ to, onClick, ...props }: { to: string } & Omit<AnchorHTMLAttributes<HTMLAnchorElement>, 'href'>) {
  return <a href={to} {...props} onClick={(event) => {
    onClick?.(event);
    if (!event.defaultPrevented && event.button === 0 && !event.metaKey && !event.ctrlKey && !event.shiftKey && !event.altKey) {
      event.preventDefault();
      window.history.pushState(null, '', to);
      window.dispatchEvent(new Event('kemuri:navigate'));
    }
  }} />;
}

type RouteProps = { path: string; element: ReactElement };
export function Route(_props: RouteProps) { return null; }

function match(pattern: string, pathname: string): Record<string, string> | null {
  const expected = pattern.split('/').filter(Boolean);
  const actual = pathname.split('/').filter(Boolean);
  if (expected.length !== actual.length) return null;
  const params: Record<string, string> = {};
  for (let index = 0; index < expected.length; index += 1) {
    const patternPart = expected[index]!;
    const actualPart = actual[index]!;
    if (patternPart.startsWith(':')) {
      params[patternPart.slice(1)] = decodeURIComponent(actualPart);
    } else if (patternPart !== actualPart) {
      return null;
    }
  }
  return params;
}

export function Routes({ children }: { children: ReactNode }) {
  const { pathname } = useContext(RouterContext);
  for (const child of Children.toArray(children)) {
    if (!isValidElement<RouteProps>(child)) continue;
    const params = match(child.props.path, pathname);
    if (params) return <RouterContext.Provider value={{ pathname, params }}>{child.props.element}</RouterContext.Provider>;
  }
  return <p role="alert">Page not found.</p>;
}

export function useParams<T extends Record<string, string | undefined>>() {
  return useContext(RouterContext).params as T;
}

export function useLocation() {
  const { pathname } = useContext(RouterContext);
  return { pathname };
}
