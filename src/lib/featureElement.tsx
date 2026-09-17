import { ReactElement } from 'react';
import { Navigate } from 'react-router-dom';

// Route element for a feature that can be disabled at build time (see features.ts):
// a disabled feature is unreachable, not only hidden from the navigation.
export function featureElement(enabled: boolean, element: ReactElement) {
  return enabled ? element : <Navigate to='/wallet' replace />;
}
