# locale-translate
This is a basic utility for translating locale files.

## Usage
Start by [setting up a DeepL API account](https://www.deepl.com/en/signup) and [generating
an API key](https://www.deepl.com/en/your-account/keys). Copy the key and use it to set the
`DEEPL_API_KEY` environment variable.

Run the tool using `cargo run --release`. You will be prompted for your desired target language, the
name of the source locale file, and the name of the output locale file. The source file must meet
two basic requirements:
1. It must be the English locale file. Other languages are currently not supported.
2. It must be a JSON file with a single object that *only* contains simple key-value pairs, and
*all* values must be strings.

Make sure to monitor your API usage to avoid running out of credit.
