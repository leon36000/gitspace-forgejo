# CONTINUATION-261 — attestation du journal avant credential du reconciler

Date : 2026-10-27. Worktree produit : `.runtime/webhook-005-workspace`; HEAD de base observé : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le dépôt était déjà très modifié avant ce lot; les manifestes listent exactement les sources touchées ici et ne prétendent pas que le commit HEAD contient leur état.

## Changement

`forgejo-reconcile-local` accepte `--trusted-webhook-attestation-public-key-hex` de façon répétée, au plus huit fois. Le parseur exige des clés Ed25519 valides, uniques, canoniques (hex minuscule de 32 octets). Si une ligne sélectionnée et non quarantinée vise le dépôt configuré et qu’un FD credential est fourni, le reconciler refuse une politique vide; avec une politique présente, il vérifie chaque ligne sélectionnée non quarantinée avant la lecture du FD. Une politique fournie impose la même vérification même sans FD. L’échec signale une cause précise et déclare `host_credential_read=false`, `upstream_calls_attempted=false`, `upstream_read_attempts=0`. Les lignes quarantinées restent bloquées séparément et ne retardent pas une autre ligne active signée.

La découverte de capacités passe à `forgejo-reconcile-capabilities-v11` et décrit le drapeau, la limite de clés, l’ordre avant credential et la compatibilité non authentifiée. Les fixtures processus qui consomment un credential ont été migrées vers le receiver réellement configuré pour signer; les tests hors portée ou avant sélection conservent leurs anciens cas sans consommation.

Le runner de processus réutilisait une ancienne signature de `compile_targets`/`isolated_command`, et copiait chaque binaire au lieu des seules cibles référencées. Il échouait avant exécution, d’abord avec `TypeError`, puis avec dépassement de l’export de 128 MiB. Il sélectionne maintenant `required_binary_targets`, passe les paramètres requis et accepte plusieurs `--scenario` afin de compiler une fois pour plusieurs cas isolés.

## Vérifications observées

- Le nouveau test processus a d’abord échoué contre le comportement antérieur : le CLI ne connaissait pas le drapeau de confiance et le test recevait un diagnostic sur stderr au lieu du refus structuré attendu.
- Sous Bubblewrap (`--unshare-net --unshare-pid`), dix scénarios processus distincts passent : politique absente; clé non approuvée; clé approuvée et lecture d’issue privée; lecture de workflow run privé; lecture de workflow job privé avec run parent; refus 401 actionnable; lot 401 puis reprise avec credential renouvelé; batch multi-dépôt; lecture d’un PR de fork; credential invalide sur PR de fork. Les deux refus d’attestation contrôlent les octets du pipe après la sortie enfant et les compteurs API.
- Tests unitaires du binaire : clé valide, politique bornée/unique/canonique; journal de webhook signé lu avec credential; batch quarantiné + événement signé actif. Les trois tests sélectionnés passent.
- `cargo check --offline --locked --all-targets` passe.
- `cargo test --offline --locked --test forgejo_reconcile_capabilities_e2e` : 5 réussis, 2 ignorés car ils réservent le port fixe 3130.
- `cargo fmt --all -- --check`, `git diff --check` et compilation Python du runner passent.
- Le garde Cargo passe, puis ses 12 tests unitaires Python passent. Cargo metadata rapporte `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits de redistribution `not_assessed` et `NOASSERTION`.

Les commandes et le détail d’environnement sont dans [commands.txt](commands.txt) et [environment.json](environment.json); les hashes des sources modifiées figurent dans [source-sha256.txt](source-sha256.txt).

Les manifestes globaux contenaient déjà des lignes vides et une entrée `None` non-SHA avant ce lot (index worktree : 6 lignes vides et 1 mal formée; index projet : 2 lignes vides). Les appenders conservent ces octets historiques et les reçus de préfixe; ils vérifient les nouvelles entrées sans réécrire ni « réparer » les anciennes.

## Limites et prochaine étape

## Retest après migration de contrat — 2026-10-27

Les réponses single/batch sont maintenant aux schémas v12/v13 et publient la politique d’attestation optionnelle; le document de capacités reste v11. Une assertion de quarantaine oubliée a été mise à jour, puis les contrats de configuration (4 tests), quarantaines (5), verrou de journal (3), capacités (5 réussis, 2 ignorés pour port fixe) et quatre tests unitaires du reconciler ont passé. `cargo check --offline --locked --all-targets` et `cargo fmt --all -- --check` passent. Après cette migration, les trois scénarios processus les plus pertinents ont été rejoués sous Bubblewrap : absence de politique et clé non approuvée sont refusées sans lire les octets du pipe; la clé approuvée autorise le chemin qui lit le credential. Les dix scénarios du premier run ne sont pas tous rejoués après la migration de schéma. Le code et les reçus sont liés par les empreintes de la révision append-only du manifeste.

Cette barrière protège uniquement le chemin qui reçoit un FD credential pour une livraison active dans le dépôt. Le rejeu sans FD et sans liste de clés reste un chemin de compatibilité non authentifié; il ne convient pas à une décision d’admission ou de merge. Le binaire reçoit les clés publiques dans argv; l’identité et l’intégrité de son lanceur ne sont pas vérifiées par lui. Le test Bubblewrap isole seulement le test. Il n’établit pas qu’un processus agent ne peut pas joindre une instance réelle ni invoquer le binaire avec sa propre clé.

Un writer local de même UID peut encore remplacer le journal autour des lectures; le rollback/tronquage, l’ancrage externe, les lignes historiques non signées, la configuration hôte protégée, la rotation/révocation et le coût de migration restent ouverts. Les signatures apportent provenance de l’attestation du receiver, pas preuve de correction logicielle. Aucun LLM/AgentOS, Forgejo live, PR/merge ou comparaison contrôlée n’a été exécuté. Le projet reste incomplet.
