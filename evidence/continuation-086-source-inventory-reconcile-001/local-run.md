# CONTINUATION-086 — rapprochement des refs sources et de Cargo

Date : 26 octobre 2026. Worktree produit : /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace, branche local/WEBHOOK-005-canonical-links, HEAD f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0. Le worktree était déjà modifié; ce lot ajoute un rapprochement documentaire et ses reçus, sans changer de code runtime ni de métadonnées Cargo.

## Refs et versions

- Commande : git ls-remote https://github.com/leon36000/GitSpace.git refs/heads/main. Résultat : dcde1142cf76996a6ba0074cb7eab9d3c870c561.
- Commande : git ls-remote https://codeberg.org/forgejo/forgejo.git refs/tags/v17.0.0 refs/tags/v17.0.0^{}. Résultat : 0d791421f85ca13a63b665053d81ea765fbbc571 pour le tag direct v17.0.0.
- La page officielle des releases Forgejo liste v17.0.0, publiée le 15 octobre 2026 et maintenue jusqu’au 28 janvier 2027 : https://forgejo.org/releases/.
- La page officielle GitHub de GitSpace sur main montre le bootstrap proposé en PR brouillon et la déclaration de licence UNKNOWN : https://github.com/leon36000/GitSpace.

Ces éléments confirment les refs déjà épinglées dans l’inventaire; le GitSpace clone d’audit antérieur est absent et n’a pas été recréé. La ref publique seule ne constitue pas un nouvel audit du tree ni un droit d’utilisation. Pour Forgejo, le tag est vérifié, mais le scratch de tests reste modifié et aucune revue exhaustive par fichier/dépendance n’est revendiquée.

## Métadonnées Cargo

- sha256sum Cargo.lock : bf599559dd11ff21ecc4754aa8a693966b4a9ca401153d7d7caa1f18d8995fcc; égal à cargo_lock_sha256 de evidence/cargo-dependency-license-inventory-001/summary.json. Le verrou correspond à l’inventaire hors ligne déjà recueilli (24 paquets, 6 dépendances directes).
- python3 scripts/verify_cargo_license_state.py : vérification réussie; license=null, license_file=null, publish=[], no_license_selected, redistribution_rights=not_assessed, SPDX export marker NOASSERTION.
- python3 -m unittest -v scripts.test_cargo_license_state : 9 tests réussis.

La licence n’est pas sélectionnée et l’autorisation de redistribution n’est pas évaluée. NOASSERTION est un marqueur personnalisé pour futur export SPDX, pas un choix Cargo ni une décision juridique.

## Limites

Aucune archive utilisateur n’a été re-cherchée dans ce lot; les limites de l’ancienne recherche sont gardées dans l’inventaire. Aucun tree GitSpace n’a été re-cloné ou audité ici; aucune dépendance n’a fait l’objet d’une nouvelle analyse juridique. Le statut de GitSpace UNKNOWN n’autorise aucune copie. Aucun LLM, AgentOS, RAG, mémoire, secret, donnée privée, dépense, production ou publication n’a été utilisé. Aucune conclusion de compatibilité ou de redistribution n’est tirée.
