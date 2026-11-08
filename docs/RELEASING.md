# Выпуск версий и подпись кода

## Как выпустить новую версию

1. Поменяйте `version` в `Cargo.toml`, например на `0.1.1`.
2. Соберите один раз локально (`cargo build`), чтобы обновился `Cargo.lock`, и сделайте коммит.
3. Поставьте тег и отправьте его на GitHub:

   ```bash
   git tag v0.1.1
   git push origin main --tags
   ```

4. Workflow **Release** на вкладке Actions соберёт exe на сервере GitHub, прогонит тесты, упакует zip с файлом `.sha256` и создаст релиз. Тег должен совпадать с версией в `Cargo.toml`, иначе сборка остановится.

Проверить сборку без релиза: вкладка Actions → **Release** → **Run workflow**. Готовый zip появится в разделе Artifacts у этого запуска.

## Подпись через SignPath Foundation

Пока exe не подписан, Windows при первом запуске показывает окно «Система Windows защитила ваш компьютер». [SignPath Foundation](https://signpath.org/) бесплатно подписывает проекты с открытым кодом. Подпись делается только для exe, собранного на GitHub, поэтому workflow **Release** уже готов к ней.

### 1. Подать заявку

Заявка подаётся на [signpath.org](https://signpath.org/). Актуальные условия смотрите там; обычно они такие:

- открытая лицензия (у нас MIT) и публичный репозиторий;
- проект уже выпущен и поддерживается;
- сборка идёт на CI, а не на компьютере разработчика;
- в README есть политика подписи кода (текст ниже) и описание того, какие данные программа передаёт по сети (раздел «Конфиденциальность» уже есть).

### 2. После одобрения

SignPath выдаст организацию и проект. В настройках репозитория на GitHub (Settings → Secrets and variables → Actions) добавьте:

| Что | Тип | Значение |
| --- | --- | --- |
| `SIGNPATH_API_TOKEN` | Secret | API-токен из SignPath |
| `SIGNPATH_ORGANIZATION_ID` | Variable | ID организации в SignPath |
| `SIGNPATH_PROJECT_SLUG` | Variable | slug проекта, например `deadlock-launcher-lite` |
| `SIGNPATH_SIGNING_POLICY_SLUG` | Variable | slug политики подписи, например `release-signing` |

В проекте SignPath укажите конфигурацию артефакта из файла [`.signpath/artifact-configuration.xml`](../.signpath/artifact-configuration.xml).

Как только появится секрет `SIGNPATH_API_TOKEN`, следующий релиз подпишется сам. Без него сборка продолжает работать и выпускает неподписанный exe.

### 3. Текст для README после одобрения

Добавьте в README раздел (SignPath просит указывать это для проектов, которые они подписывают):

```markdown
## Политика подписи кода

Бесплатная подпись кода предоставлена [SignPath.io](https://about.signpath.io/), сертификат — [SignPath Foundation](https://signpath.org/).

- Авторы и проверяющие: [участники репозитория](https://github.com/sorokin2486/deadlock-launcher-lite/graphs/contributors)
- Подтверждает выпуск: [sorokin2486](https://github.com/sorokin2486)
```
