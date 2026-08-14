import { Suspense } from "react";
import { CredentialsForm } from "../CredentialsForm";

// `CredentialsForm` calls `useSearchParams` (to read the `next` param that `proxy.ts` sets),
// which needs a Suspense boundary above it — without one the whole route opts into
// client-side rendering and the build warns about it.
export default function LoginPage() {
  return (
    <Suspense>
      <CredentialsForm mode="login" />
    </Suspense>
  );
}
