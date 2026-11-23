import { createContext, useContext, useMemo } from 'react';

export const AppContext = createContext(null);
export const useApp = () => useContext(AppContext);

export function useRouteReadiness(feature) {
  const { bindRoute } = useApp();
  return useMemo(() => bindRoute(feature), [bindRoute, feature]);
}
