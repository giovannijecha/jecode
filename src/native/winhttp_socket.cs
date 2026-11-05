// One WebSocket connection per helper process, selected by the `socket` argument.
// Stdin: base64 URL, header count, base64 name/value pairs, then for each message
// its byte length on a line and its UTF-8 bytes. Closing stdin ends the helper.
// Stdout, in the frames of winhttp.cs: 'A' once connected, 'D' per whole message,
// and 'E' with the reason when the connection ends.
using System;
using System.IO;
using System.Globalization;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public static partial class JecodeHttp {
    const uint UpgradeToWebSocket = 114;
    const uint StatusCodeNumber = 19 | 0x20000000;
    const uint BinaryMessage = 0;
    const uint Utf8Message = 2;
    const uint CloseMessage = 4;
    const uint SwitchingProtocols = 101;

    [DllImport("winhttp.dll", SetLastError = true)]
    static extern bool WinHttpSetOption(IntPtr handle, uint option, IntPtr value, uint size);
    [DllImport("winhttp.dll", SetLastError = true)]
    static extern bool WinHttpQueryHeaders(IntPtr request, uint info, IntPtr name,
        ref uint value, ref uint size, IntPtr index);
    [DllImport("winhttp.dll", SetLastError = true)]
    static extern IntPtr WinHttpWebSocketCompleteUpgrade(IntPtr request, IntPtr context);
    [DllImport("winhttp.dll")]
    static extern uint WinHttpWebSocketSend(IntPtr socket, uint type, byte[] buffer, uint length);
    [DllImport("winhttp.dll")]
    static extern uint WinHttpWebSocketReceive(IntPtr socket, byte[] buffer, uint length,
        out uint read, out uint type);

    static int ServeSocket() {
        using (var input = new BufferedStream(Console.OpenStandardInput()))
        using (var output = Console.OpenStandardOutput()) {
            try {
                Uri uri = new Uri(DecodeText(Required(input)), UriKind.Absolute);
                int count = Number(Required(input));
                if (count > 1000) throw new InvalidDataException("Native HTTP request is invalid");
                var exchange = new Exchange();
                for (int i = 0; i < count; i++) {
                    string name = DecodeText(Required(input));
                    string value = DecodeText(Required(input));
                    if (!Token(name) || value.IndexOfAny(new char[] { '\r', '\n', '\0' }) >= 0)
                        throw new InvalidDataException("Native HTTP request header is invalid");
                    exchange.Headers.Add(new string[] { name, value });
                }
                IntPtr socket = Upgrade(uri, exchange);
                Frame(output, 'A', null, 0);
                var sender = new Thread(() => SendMessages(input, socket));
                sender.IsBackground = true;
                sender.Start();
                ReceiveMessages(socket, output);
            } catch (Exception error) {
                byte[] message = Encoding.UTF8.GetBytes(Describe(error));
                try {
                    Frame(output, 'E', message, message.Length);
                } catch (IOException) {
                    // The parent stopped reading; nothing is left to report to.
                }
            }
        }
        return 1;
    }

    static IntPtr Upgrade(Uri uri, Exchange exchange) {
        // HTTP/1.1 is the session default, which the upgrade requires.
        IntPtr session = WinHttpOpen(null, uri.IsLoopback ? NoProxy : AutomaticProxy, null, null, 0);
        Require(session, "open");
        IntPtr connection = WinHttpConnect(session, uri.Host, checked((ushort)uri.Port), 0);
        Require(connection, "connect");
        IntPtr request = WinHttpOpenRequest(connection, "GET", uri.PathAndQuery, IntPtr.Zero,
            IntPtr.Zero, IntPtr.Zero, uri.Scheme == "https" ? Secure : 0);
        Require(request, "request");
        // The parent bounds the upgrade and silence; messages may arrive minutes apart.
        Check(WinHttpSetTimeouts(request, 10000, 10000, 30000, 0), "timeouts");
        uint disabled = DisableCookiesRedirectsAuth;
        Check(WinHttpSetOption(request, DisableFeature, ref disabled, sizeof(uint)), "features");
        if (uri.Scheme == "https") {
            uint revocation = EnableSslRevocation;
            Check(WinHttpSetOption(request, EnableFeature, ref revocation, sizeof(uint)), "revocation");
        }
        Check(WinHttpSetOption(request, UpgradeToWebSocket, IntPtr.Zero, 0), "upgrade");
        foreach (string[] header in exchange.Headers) {
            Check(WinHttpAddRequestHeaders(request, header[0] + ": " + header[1] + "\r\n",
                UInt32.MaxValue, AddHeader), "header");
        }
        Check(WinHttpSendRequest(request, null, 0, IntPtr.Zero, 0, 0, IntPtr.Zero), "send");
        Check(WinHttpReceiveResponse(request, IntPtr.Zero), "receive");
        uint status = 0, size = sizeof(uint);
        Check(WinHttpQueryHeaders(request, StatusCodeNumber, IntPtr.Zero, ref status, ref size, IntPtr.Zero),
            "status");
        if (status != SwitchingProtocols)
            throw new IOException("Native HTTP socket was refused (HTTP " + status + ")");
        IntPtr socket = WinHttpWebSocketCompleteUpgrade(request, IntPtr.Zero);
        Require(socket, "upgrade");
        WinHttpCloseHandle(request);
        return socket;
    }

    // Owns stdin, so a parent that exits ends the helper even while it waits for a message.
    static void SendMessages(Stream input, IntPtr socket) {
        try {
            for (;;) {
                string line = Line(input);
                if (line == null) Environment.Exit(0);
                byte[] message = new byte[Number(line)];
                for (int filled = 0; filled < message.Length;) {
                    int read = input.Read(message, filled, message.Length - filled);
                    if (read <= 0) Environment.Exit(0);
                    filled += read;
                }
                uint error = WinHttpWebSocketSend(socket, Utf8Message, message, (uint)message.Length);
                // Closing the handle fails the receive that reports to the parent.
                if (error != 0) break;
            }
        } catch (Exception) {
            // Reported the same way below.
        }
        WinHttpCloseHandle(socket);
    }

    static void ReceiveMessages(IntPtr socket, Stream output) {
        byte[] buffer = new byte[65536];
        var message = new MemoryStream();
        for (;;) {
            uint read, type;
            uint error = WinHttpWebSocketReceive(socket, buffer, (uint)buffer.Length, out read, out type);
            if (error != 0) throw new IOException("Native HTTP socket failed (" + error + ")");
            if (type == CloseMessage) throw new IOException("Native HTTP socket was closed");
            message.Write(buffer, 0, (int)read);
            if (type == Utf8Message || type == BinaryMessage) {
                Frame(output, 'D', message.GetBuffer(), (int)message.Length);
                message.SetLength(0);
            }
        }
    }
}
