# Mission 012 composé 046 — admission webhook, changement, PR et fusion locale

Date : 25 octobre 2026. Worktree : `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, checkout fortement modifié. Le composeur a capturé 83 entrées (2 579 559 octets), archive SHA-256 `b42217acd460876f947130d3486a750792091e52ae02294451522268660939b2`; le source-map SHA-256 est `da86219a889ec649cb6298d41bcb80552150f003ec72cd575dc7523f7d27103c`. Le check de sources a réussi avant la copie du snapshot.

## Parcours réalisé

Commande :

```sh
python3 scripts/run_mission_012_live_composed_e2e.py \
  --state-dir /tmp/mission012-composed-adapter-046 \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --evidence-dir evidence/mission-012-live-agent-webhook-local-adapter-046 \
  --execution-id live-mcp-046 \
  --allocation-id mission012-webhook-live-046 \
  --budget-cap-usd 0.01 \
  --agent-adapter local-test-adapter
```

Le processus a quitté avec le code 0. Une instance Forgejo 17.0.0 privée et temporaire a écouté sur `127.0.0.1:3132`; sa base, son dépôt, le receiver et les reçus sont sous `/tmp/mission012-composed-adapter-046`. La livraison `issues/opened` est liée à l’issue #1 et au dépôt privé `forge-admin/mission-012-clamp`. L’exigence `MISSION-012-CLAMP-001` est liée à son SHA-256 `3a3c444075c93a3bcfb2bec17d1a2e74ec0051effc2994e6b02ac095eb0dd431`, à l’allocation `mission012-webhook-live-046` et au commit de base `c64d94fbdb488f4f88c14b5d2c29ba3ce34cdb0b`. Le credential hôte invalide est refusé `mission_source_unauthorized`; la rediffusion de l’allocation est refusée `already_claimed`.

L’exécutant était `local-test-adapter` en mode déterministe : aucun LLM, zéro token mesuré, coût déclaré 0 USD. Le plafond 0,01 USD ne constitue pas une consommation de modèle. L’adaptateur a produit une seule modification, `src/lib.rs`, vers le SHA `173304ad7d32aabc1e8e1ee41eabda3398a8690798e68f906bcebc73cbcd6d15`, puis la PR #2, commit candidat `224fa6e0efddea0044d14d863642ef66f912b017`.

## Échec, correction et contrôle de fusion

Le vérificateur indépendant a d’abord reproduit le défaut sur la base : `values_outside_the_range_clamp_to_the_nearest_boundary` échoue (`left: -2`, `right: 10`; 1 réussi, 1 échoué; code 101). Le candidat passe les deux tests d’acceptation, puis une nouvelle vérification pré-fusion passe les mêmes deux tests sur le même commit candidat. L’environnement de vérification utilise Cargo hors ligne verrouillé, un snapshot du commit en lecture seule, Bubblewrap avec namespace réseau isolé, cible Cargo tmpfs, sans home ni credential hôte; `systemd-run` applique la portée de ressources externe au candidat. Le journal CPU indique 18 464 ms imputés sur un budget de mission de 1 200 000 ms; ce nombre est une borne comptable calculée depuis le quota et le temps, pas une télémétrie CPU directe.

La tentative de fusion avant revue est refusée HTTP 405; celle du reviewer non autorisé l’est aussi. Après la revue `verifier-bot` liée au candidat exact, l’autorité Rust locale `forgejo-reconcile-local-rust` fusionne la PR par le commit `ee3fd74ad24f715d2aebed86ca0de3ea6c780a18`. Le readback Forgejo indique `main` sur ce commit, issue et PR fermées. Les parents du commit de merge sont exactement la base et le candidat et son arbre est identique à celui du candidat. Trois événements PR (ouverte, revue approuvée, fermée) ont reçu HTTP 202 puis ont été réconciliés à l’état courant. Le hook est désactivé, le receiver arrêté et son port contrôlé absent.

## Vérification comportementale après fusion

Après la fusion, un contrôle distinct a matérialisé le commit `main` exact depuis le dépôt bare privé, vérifié ses parents et l’égalité arbre/candidat, puis exécuté `cargo test --offline --locked` sur ce snapshot exact. Résultat : code 0, 2 tests d’acceptation réussis, 0 échec, 371 ms; Bubblewrap isole réseau, montages et processus et `systemd-run` impose les limites de ressources. Le reçu [postmerge-verification.json](postmerge-verification.json) est rétrospectif et n’ajoute aucune autorité ni écriture au journal de mission.

## Forgejo vérifié

Le binaire arm64 a été téléchargé depuis le miroir officiel, sa signature `.asc` vérifiée avec GPG et l’empreinte SHA-256 comparée au pin du runner : `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Le primaire `EB114F5E6C0DC2BCDD183550A4B61A2DC5923710` concorde avec l’empreinte publiée par Forgejo; GPG indique que la confiance locale de la clé reste inconnue. Le binaire annonce `17.0.0+gitea-1.22.0`. Cela vérifie l’artefact de release signé, pas une compilation reproductible depuis le commit source audité. Voir [la provenance](forgejo-binary-provenance.json), [la signature archivée](forgejo-17.0.0-linux-arm64.asc) et [les versions](versions.txt).

## Portée et limites

Ce run prouve un parcours composé sur les sources capturées de ce worktree, avec le binaire Forgejo officiel local. Il ne prouve pas un parcours LLM, une intégration AgentOS/RAG/mémoire, une comparaison Forgejo+agent+CI conventionnelle, ni un gain de qualité, de coût, de délai ou de corrections humaines. C’est une seule tâche et un seul run; les tokens et coûts du modèle ne sont pas mesurés. L’autorité locale s’exécute avec le credential owner synthétique de la forge de test; le reçu précise que l’exclusion des writers externes n’est pas fournie par un lock coopératif. Le vérificateur rétrospectif ne remplace pas la preuve pré-fusion. Les autres scénarios de refus doivent rester évalués selon leurs propres preuves.

Aucun code Forgejo ou GitSpace n’a été copié au produit; aucune production, publication, suppression, extension d’accès ou dépense de modèle n’a eu lieu. Le commit d’audit Forgejo demeure `0d791421f85ca13a63b665053d81ea765fbbc571`; la licence produit Cargo n’est pas choisie.
