# CONTINUATION-392 — action exploitable pour un nom de branche invalide

Date : 28 octobre 2026. Worktree `local/WEBHOOK-005-canonical-links`, base HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Défaut et correction

Le validator d’ingress vérifie déjà `payload.ref` avec `crate::forgejo::valid_branch`, avant tout append à l’inbox. Un nom invalide recevait néanmoins l’action HTTP générique. Le scénario `actual_receiver_refuses_invalid_create_branch_ref_with_actionable_guidance` transmet `.hidden/release` comme branche de `create`; avant la modification, il échoue sur l’action générique après avoir confirmé le refus et le chemin `payload.ref`.

Le mapping HTTP indique maintenant qu’il faut fournir un nom de branche Git valide, avec composants non vides ne commençant pas par un point, sans slash/point final et sans métacaractères de référence. Il lie le conseil au endpoint de capacités et à la redelivery avec le même delivery ID. La règle d’admission et les capacités versionnées n’ont pas changé.

## Résultat

Le vrai processus `forgejo-webhook-local` passe 1/1 sous Bubblewrap `--unshare-net --unshare-pid` (186 tests filtrés). Il retourne HTTP 400, `event_payload_invalid`, `retryable=false`, `details.field_path=payload.ref`, l’action spécialisée, et laisse l’inbox vide. `cargo fmt --all -- --check`, `git diff --check`, l’AST du runner et la garde de métadonnées Cargo passent.

Cargo demeure inchangé : `license=null`, `license_file=null`, `publish=[]`, SPDX `NOASSERTION`, droits de redistribution non évalués. Le run utilise Rust/Cargo 1.95.0, Linux aarch64 et Bubblewrap 0.9.0. Commandes et sorties rouge/verte dans ce répertoire.

## Portée

Cette preuve porte sur un nom invalide représentatif et la qualité du diagnostic. Elle ne démontre pas une validation exhaustive de toutes les règles Git-ref, une émission native Forgejo, un LLM/AgentOS, une PR/fusion ou un gain comparatif. Aucun réseau externe, courriel ou accès GitLab n’a été utilisé. Le projet reste incomplet.
