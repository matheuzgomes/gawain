import json


class Decoder(json.JSONDecoder):
    LEVEL = 3  # expect: GW004

    def __init__(self) -> None:
        self.strict = True
