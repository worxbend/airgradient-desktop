# AirGradient Local Server Notes

Air Monitor reads the AirGradient local-server endpoint:

```text
GET /measures/current
```

For a configured base URL of `http://192.168.1.201`, the app requests:

```text
http://192.168.1.201/measures/current
```

## URL Normalization

Users can enter:

```text
192.168.1.201
http://192.168.1.201
http://192.168.1.201/
http://192.168.1.201:80
```

The app normalizes these into a base URL with:

- an explicit `http` or `https` scheme
- no path
- no query string
- no fragment
- no trailing slash in the saved value

## Payload Handling

Different AirGradient models and firmware versions can expose slightly different JSON keys. The parser therefore looks for multiple candidate names for the same concept.

Examples:

| Meaning | Keys |
| --- | --- |
| CO2 | `rco2`, `co2`, `co2_ppm` |
| PM2.5 | `pm02`, `pm2_5`, `pm25`, `pm2.5` |
| temperature | `atmpCompensated`, `atmp`, `temperature`, `temp_c` |
| humidity | `rhumCompensated`, `rhum`, `humidity`, `rh` |
| TVOC | `tvocIndex`, `tvoc`, `tvoc_ppb` |
| NOx | `noxIndex`, `nox`, `nox_ppb` |

The parser also searches nested JSON objects, which makes it more tolerant of payloads that group measurements under another object.

## AQI

If the payload includes `aqi` or `air_quality_index`, that value is used.

If AQI is missing but PM2.5 is present, the app estimates US AQI from PM2.5 breakpoints. This is a display convenience, not a replacement for official regulatory AQI calculations.

## Missing Values

Missing values are represented as `None` in Rust and shown as `--` in the UI.

The app does not treat missing values as zero because zero is a real measurement value.
