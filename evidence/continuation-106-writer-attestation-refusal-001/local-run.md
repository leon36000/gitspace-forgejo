# CONTINUATION-106 — refus du profil single-writer sans attestation

Date : 26 octobre 2026. Base Git inspectée : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le worktree était déjà fortement modifié; aucune remise à zéro, suppression, publication, modification de production, lecture de secret, ni action sur un processus externe n’a été faite.

## Constat

CONTINUATION-097 avait laissé `local_single_writer_sandbox` à la discrétion du host. L’audit du checkpoint confirme seulement l’isolation réseau du child agent; il n’établit pas que Forgejo lui-même est inaccessible aux autres writers du host pendant l’admission, le contrôle et le merge. Forgejo v17 ne fournit pas de précondition atomique de base sur tous les chemins de merge. Sans l’une de ces deux garanties, accepter l’option single-writer serait une déclaration non vérifiée.

## Changement local

- `validate_merge_writer_admission_v1` exige maintenant un signal de confiance d’exclusion des writers pour le profil local; le profil multi-writer reste refusé sans CAS serveur.
- Le seul chemin de CLI de production transmet explicitement `false`, car aucun superviseur ne produit ni ne vérifie encore une attestation de frontière complète. La sélection explicite de l’ancien profil retourne `inconclusive/single_writer_supervisor_attestation_unavailable`, `merge_authorized=false`, `merge_attempted=false`.
- Les fixtures Rust en mémoire peuvent transmettre `true` pour exercer les branches de gate aval. Ce booléen n’authentifie rien et ne constitue pas une attestation; aucun chemin de CLI n’accepte une preuve positive à ce jour.
- Le test préexistant qui s’attendait à atteindre le lecteur de credential avec le profil local a été remplacé par un test qui confirme le refus préalable et que le contenu du pipe credential reste intact.
- Les documents d’architecture, checkpoint et matrice ont été mis à jour pour signaler le blocage et ne plus qualifier le chemin antérieur de reproductible sous l’admission courante.

## Vérifications observées

- `cargo test --offline --locked --test forgejo_merge_writer_admission` : 2/2.
- `cargo test --offline --locked --bin forgejo-reconcile-local merge_cli_refuses_single_writer_profile_without_supervisor_attestation_before_credential` : 1/1.
- `cargo test --offline --locked --bin forgejo-reconcile-local valid_single_writer_selector_refuses_before_reaching_the_credential_reader` : 1/1; le FD pipe garde les octets synthétiques après refus.
- `cargo test --offline --locked --all-targets` : sortie 0 après mise à jour de l’ancien test. Le premier run avait échoué parce que celui-ci attendait le code d’erreur du lecteur; le nouveau refus arrive avant ce lecteur. Le run complet n’a pas été relancé pour produire un journal plus compact; les comptes agrégés ne sont donc pas revendiqués. La compilation/test a été répétée sur les deux cas ciblés et les logs sont conservés.
- Le vrai binaire local `target/debug/forgejo-reconcile-local`, avec `--merge-local-test --writer-admission-profile local_single_writer_sandbox` et FD `999999`, retourne code 1 avec le code d’erreur attendu et sans stderr. Il ne lit pas ce FD; la sortie et le code sont conservés ici. Le refus arrive avant le credential dans le flux CLI. Aucun serveur Forgejo n’a été sollicité dans cette expérience; le résultat n’établit pas par capture réseau l’absence universelle de requêtes.
- `cargo fmt --all -- --check`, `git diff --check` et `cargo metadata --offline --locked --no-deps` : code 0.

Commandes et extraits : `verification-results.json`, les journaux ciblés et `cli-single-writer-refusal.stdout.json`. Les SHA-256 des sources et documents changés sont dans `source-sha256.json`.

## Portée et suite

C’est une fermeture fail-closed de la CLI, pas l’implémentation d’une attestation positive. Le booléen de test est volontairement non assimilé à une preuve de provenance. Aucun serveur live, LLM/AgentOS, nouveau coût, service externe ou changement Forgejo amont n’a été utilisé. La fusion MISSION-012 précédente reste une observation historique de son run; ce chemin de fusion n’est pas reproductible avec la nouvelle politique tant qu’un superviseur ne fournit pas l’attestation manquante. Le projet demeure incomplet.

Prochaine action : démontrer si un superviseur local peut lancer Forgejo et le gate dans une frontière réseau/processus commune empêchant effectivement les writers externes et produire un reçu vérifiable. Si cette exclusion ne peut pas être imposée sur cet hôte, conserver le refus. La CAS serveur, l’agent/runtime autorisé, les répétitions de parcours, la comparaison conventionnelle et les droits GitSpace restent ouverts.
