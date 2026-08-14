import { Suspense } from "react";
import { CredentialsForm } from "../CredentialsForm";

// See the note in `login/page.tsx` on why the Suspense boundary is here.
export default function RegisterPage() {
  return (
    <Suspense>
      <CredentialsForm mode="register" />
    </Suspense>
  );
}
