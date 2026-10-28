# MISSION-012-POSTMERGE-FRESH-CHECKOUT-001 — contrôle depuis un clone frais

Date : 25 octobre 2026. Forgejo local temporaire : `17.0.0+gitea-1.22.0`, binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Ce lot reprend les objets Git conservés par l’E2E 062; aucun serveur ni service n’a été démarré ou modifié pendant cette vérification.

## Procédure

Depuis le dépôt nu du bac à sable Forgejo `/tmp/mission-012-forgejo-restart-062/repositories/forge-admin/mission-012-clamp.git`, j’ai fait un clone local neuf avec `git clone --no-hardlinks --no-checkout`, puis des checkouts détachés explicites de la base `5e621a3d593eb908b8ac4e3fe9e6d2d5f8bc1828` et du commit fusionné `51860e96e265be51194e505429914af0e6512c9b`. Le commit de fusion a pour parents la base et la candidate PR `ab59d515e6c6f6c3122eb59e766661de581ad9f2`; son arbre Git `518f1e74cfd70f848c882b9937588dadd5710b10` est identique à l’arbre candidat. Le worktree était propre aux deux checkouts.

À chaque checkout, j’ai exécuté `cargo test --offline --locked --test acceptance` sous un scope systemd limité (mémoire 1 GiB, 64 tâches, quota CPU 200 %, durée maximale 120 s) et Bubblewrap (espaces de noms utilisateur/montage/PID/réseau, sources en lecture seule, cible Cargo et home vierges en tmpfs, aucun home hôte ni credential). Rust `1.95.0 (59807616e 2026-04-14)`, Cargo `1.95.0 (f2d3ce0bd 2026-03-21)`, Linux aarch64. Le réseau était désactivé et Cargo verrouillé en mode hors ligne.

## Résultats

- Base : statut Cargo 101, `1 passed; 1 failed`. Le test ciblé échoue avec le défaut attendu : `left: -2`, `right: 10`.
- Commit fusionné exact : statut 0, `2 passed; 0 failed`. Le fichier `src/lib.rs` a le SHA-256 `173304ad7d32aabc1e8e1ee41eabda3398a8690798e68f906bcebc73cbcd6d15`, et le fichier d’acceptation est inchangé (`5d4727542ea32f61aa4500754556fcfd1f39b59b4131ede257cb736fab231dc2`).

Les sorties et le readback des commits sont dans [base-acceptance.log](base-acceptance.log), [merge-acceptance.log](merge-acceptance.log) et [postmerge-fresh-checkout.json](postmerge-fresh-checkout.json). Le manifeste local valide ces fichiers.

## Limites

Cette vérification établit que les octets du commit de fusion présents dans le dépôt Git local persistent et passent l’acceptation après clone frais. Elle ne reteste pas le serveur Forgejo/API; le lien avec Forgejo vient du dépôt nu et du readback API de l’E2E 062. C’est le petit projet clamp du bac à sable, un parcours déterministe `local-test-adapter` (`n=1`), pas un parcours LLM ni un comparatif de productivité. Aucun code produit n’a changé; Cargo reste sans licence sélectionnée (`license=null`, `license_file=null`, `publish=[]`, décision `unselected`, droits `not_assessed`).
