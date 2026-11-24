#!/usr/bin/env python3
"""
Lightweight SwyxWare ConfigDataStore (CDS) SOAP Mock Server.
Listens on port 8094 and responds with authentic XML SOAP payloads.
"""
from http.server import HTTPServer, BaseHTTPRequestHandler
import sys

class SwyxCdsMockHandler(BaseHTTPRequestHandler):
    def do_POST(self):
        content_length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(content_length).decode("utf-8", errors="ignore")
        soap_action = self.headers.get("SOAPAction", "")

        print(f"[Swyx CDS Mock] Request: {self.path}, SOAPAction: {soap_action}", flush=True)

        resp_xml = ""

        if "Login" in soap_action or "<Login" in body:
            resp_xml = """<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
  <s:Body>
    <LoginResponse xmlns="http://tempuri.org/">
      <LoginResult xmlns:a="http://schemas.datacontract.org/2004/07/IpPbx.Web.Model">
        <a:AccessToken>mock_jwt_access_token_swyx_12345</a:AccessToken>
        <a:RefreshToken>mock_jwt_refresh_token_swyx_67890</a:RefreshToken>
        <a:UserId>42</a:UserId>
      </LoginResult>
    </LoginResponse>
  </s:Body>
</s:Envelope>"""

        elif "GetSipCredentials" in soap_action or "<GetSipCredentials" in body:
            resp_xml = """<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
  <s:Body>
    <GetSipCredentialsResponse xmlns="http://tempuri.org/">
      <GetSipCredentialsResult xmlns:a="http://schemas.datacontract.org/2004/07/SWConfigDataWSLib.Transport">
        <a:SipRealm>127.0.0.1</a:SipRealm>
        <a:SipUserID>101</a:SipUserID>
        <a:SipUserName>1001</a:SipUserName>
        <a:UserID>42</a:UserID>
      </GetSipCredentialsResult>
    </GetSipCredentialsResponse>
  </s:Body>
</s:Envelope>"""

        elif "GetCurrentPSKEx" in soap_action or "<GetCurrentPSKEx" in body:
            resp_xml = """<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
  <s:Body>
    <GetCurrentPSKExResponse xmlns="http://tempuri.org/">
      <GetCurrentPSKExResult>secret1001</GetCurrentPSKExResult>
    </GetCurrentPSKExResponse>
  </s:Body>
</s:Envelope>"""

        elif "GetNumbers" in soap_action or "<GetNumbers" in body:
            resp_xml = """<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
  <s:Body>
    <GetNumbersResponse xmlns="http://tempuri.org/">
      <GetNumbersResult xmlns:a="http://schemas.datacontract.org/2004/07/SWConfigDataWSLib.Transport">
        <a:UserNumberInfo>
          <a:InternalNumber>1001</a:InternalNumber>
          <a:PublicNumber>+49231123456</a:PublicNumber>
          <a:IsPrimary>true</a:IsPrimary>
        </a:UserNumberInfo>
      </GetNumbersResult>
    </GetNumbersResponse>
  </s:Body>
</s:Envelope>"""

        elif "GetRichPresenceInfo" in soap_action or "<GetRichPresenceInfo" in body:
            resp_xml = """<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
  <s:Body>
    <GetRichPresenceInfoResponse xmlns="http://tempuri.org/">
      <GetRichPresenceInfoResult xmlns:a="http://schemas.datacontract.org/2004/07/SWConfigDataWSLib.Transport">
        <a:PresenceStatus>0</a:PresenceStatus>
        <a:StatusText>Available</a:StatusText>
      </GetRichPresenceInfoResult>
    </GetRichPresenceInfoResponse>
  </s:Body>
</s:Envelope>"""

        elif "GetCallJournal" in soap_action or "<GetCallJournal" in body:
            resp_xml = """<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
  <s:Body>
    <GetCallJournalResponse xmlns="http://tempuri.org/">
      <GetCallJournalResult xmlns:a="http://schemas.datacontract.org/2004/07/SWConfigDataWSLib.Transport">
        <a:CallJournalEntry>
          <a:CallerNumber>+491701234567</a:CallerNumber>
          <a:CalledNumber>1001</a:CalledNumber>
          <a:CallerName>Max Mustermann</a:CallerName>
          <a:Direction>Inbound</a:Direction>
          <a:Timestamp>2026-10-12T14:30:00Z</a:Timestamp>
        </a:CallJournalEntry>
        <a:CallJournalEntry>
          <a:CallerNumber>1001</a:CallerNumber>
          <a:CalledNumber>+491709876543</a:CalledNumber>
          <a:CallerName>Erika Musterfrau</a:CallerName>
          <a:Direction>Outbound</a:Direction>
          <a:Timestamp>2026-10-12T15:15:00Z</a:Timestamp>
        </a:CallJournalEntry>
      </GetCallJournalResult>
    </GetCallJournalResponse>
  </s:Body>
</s:Envelope>"""

        else:
            resp_xml = """<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Body><SuccessResponse /></s:Body></s:Envelope>"""

        resp_bytes = resp_xml.encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", 'text/xml; charset="utf-8"')
        self.send_header("Content-Length", str(len(resp_bytes)))
        self.end_headers()
        self.wfile.write(resp_bytes)

    def log_message(self, format, *args):
        pass

def main():
    port = 8094
    server = HTTPServer(("0.0.0.0", port), SwyxCdsMockHandler)
    print(f"Swyx CDS Mock Server listening on http://0.0.0.0:{port}...", flush=True)
    server.serve_forever()

if __name__ == "__main__":
    main()
