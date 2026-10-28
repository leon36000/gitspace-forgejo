# CONTINUATION-187-FULL — parcours composé avec observation du contexte

Date : 27 octobre 2026. Worktree `.runtime/webhook-005-workspace`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Un run frais du parcours composé a été fait après l’ajout du signal d’observation CONTINUATION-187. Le worktree était fortement modifié; aucun reset, commit, publication, accès externe ni changement de production n’a été effectué.

## Commande et environnement

```sh
python3 scripts/run_mission_012_supervised_private_netns_e2e.py \
  --state-dir /tmp/mission-012-cont187-context-observation \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --evidence-dir "$PWD/evidence/mission-012-live-agent-webhook-cont187-context-observation" \
  --execution-id live-mcp-187 \
  --allocation-id mission012-cont187-context-observation
```

Le run externe rapporte `verdict=passed`, sortie interne 0, aucun délai expiré, et tous ses checks nommés vrais. Il a utilisé Bubblewrap 0.9.0, une instance Forgejo privée et locale v17.0.0+gitea-1.22.0 (binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`) dans un namespace réseau différent de l’hôte. L’adaptateur était `local-test-adapter`; aucun LLM n’a été invoqué (`model_invoked_during_run=false`, coût déclaré par l’adaptateur 0 USD; tokens non applicables). Les artefacts d’exécution et leurs empreintes locales sont dans ce dossier. Les pièces webhook/admission séparées sont dans [le dossier d’admission](../mission-012-live-agent-webhook-cont187-context-observation-admission/local-run.md).

## Résultat observable

- L’admission version 2 lie l’exigence SHA-256 `3a3c444075c93a3bcfb2bec17d1a2e74ec0051effc2994e6b02ac095eb0dd431` et une enveloppe `untrusted_work_item` issue de `current_forgejo_issue_api`, marquée `untrusted_user_content`. Le résumé ne recopie pas le titre ni le corps.
- Les reçus agent listent `mission.context`; `composed-stage-summaries.json` contient `test_adapter_untrusted_work_item_observed=true`. Le gate de composition lie ce booléen à la présence du DTO admis. Cette observation provient toutefois du code déterministe du test-adapter; elle ne prouve pas ce qu’un LLM aurait lu ou compris.
- La base échoue sur le test d’acceptation de valeur hors limites : 1 réussi, 1 échoué. Le candidat `6e9e135b1fef0f32c6b6e6159b783dde44812410` passe 2/2, comme la revérification pré-merge.
- La PR locale #2 a été refusée avant revue (HTTP 405), puis fusionnée après vérification indépendante (`pass`) au commit `f5f94e57785308faf5776a6c433c57d8bb79da74` (HTTP 200). Un reviewer non autorisé est aussi refusé (HTTP 405). La tête relue de `main` correspond au commit fusionné; le tree merge égale le tree candidat.
- La vérification post-merge est liée au même commit et tree, exécute `cargo test --offline --locked --test acceptance` dans un snapshot en lecture seule sous Bubblewrap hors réseau, et donne `2 passed; 0 failed`.
- Le processus agent sandboxé rapporte `credential_visible=false`, `network_connected=false`, zéro connexion de route observée, et les sondes directes `api`, `web`, `git_receive_pack` bloquées. Le credential de fusion est passé uniquement à l’exécution du gate. Le rapport de supervision confirme les contrôles de namespace, la séparation du réseau Forgejo et le refus de jonction de namespace non privilégiée.

## Portée

C’est un seul run composé déterministe dans un bac local privé, pas une mesure de productivité ni un benchmark. Le signal de consommation est un booléen du test-adapter et le run n’utilise ni LLM, ni AgentOS, ni intégration RAG/mémoire. Les vérifications réseau couvrent le processus sandboxé et les sondes/règles consignées; elles n’établissent pas la résistance à l’exécution privilégiée, aux autres identités du même hôte, à toutes les routes/writers ou à un environnement de production. Une empreinte atteste l’identité du reçu, pas la correction générale du logiciel. Les travaux comparatifs appariés, effectifs et coûts facturés restent absents.

## Preuves et intégrité

Les reçus nommés sont : `composed-run.json`, `composed-stage-summaries.json`, `forgejo-result.json`, `independent-verifier.json`, `postmerge-verification.json`, `supervised-private-netns-observation.json` et les journaux Cargo. `source-hashes.txt` lie les sources d’instrumentation et d’exécution; `toolchain.txt` note les versions. Le préfixe append-only antérieur est conservé dans `manifest-prefix-before.json`. Les scripts d’append et de vérification sont inclus dans ce dossier; `manifest-check.txt` et `global-manifest-check.txt` enregistrent leur résultat final.
