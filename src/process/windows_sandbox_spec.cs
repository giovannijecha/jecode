// Owned encoder for the public ProcessSecurityEnvironment.fbs 1.1 ABI.
// Only the fields Jecode needs are encoded; every offset is written forward.
internal static class JecodeWindowsSandboxSpec {
    sealed class Buffer {
        readonly System.Collections.Generic.List<byte> bytes = new System.Collections.Generic.List<byte>();

        public Buffer() {
            Put32(0); // Root table offset, patched after construction.
            bytes.AddRange(new byte[] { (byte)'P', (byte)'S', (byte)'E', (byte)'C' });
        }
        public int Length { get { return bytes.Count; } }
        public void Align(int boundary) {
            while (bytes.Count % boundary != 0) bytes.Add(0);
        }
        public void Put16(int value) {
            bytes.Add((byte)value); bytes.Add((byte)(value >> 8));
        }
        public void Put32(int value) {
            Put16(value); Put16(value >> 16);
        }
        public void Patch32(int index, int value) {
            bytes[index] = (byte)value;
            bytes[index + 1] = (byte)(value >> 8);
            bytes[index + 2] = (byte)(value >> 16);
            bytes[index + 3] = (byte)(value >> 24);
        }
        public void PatchOffset(int index, int target) {
            if (target <= index) throw new System.InvalidOperationException("FlatBuffer offset is not forward");
            Patch32(index, checked(target - index));
        }
        public void PatchByte(int index, byte value) { bytes[index] = value; }
        public int Table(int size, params int[] fields) {
            Align(2);
            int vtable = Length;
            Put16(checked(4 + fields.Length * 2));
            Put16(size);
            foreach (int field in fields) Put16(field);
            Align(4);
            int table = Length;
            Put32(table - vtable);
            for (int i = 4; i < size; i++) bytes.Add(0);
            return table;
        }
        public int String(string value) {
            Align(4);
            int start = Length;
            byte[] utf8 = System.Text.Encoding.UTF8.GetBytes(value);
            Put32(utf8.Length);
            bytes.AddRange(utf8);
            bytes.Add(0);
            return start;
        }
        public int Strings(string[] values) {
            Align(4);
            int start = Length;
            Put32(values.Length);
            foreach (string ignored in values) Put32(0);
            for (int i = 0; i < values.Length; i++)
                PatchOffset(start + 4 + i * 4, String(values[i]));
            return start;
        }
        public byte[] Finish(int root) {
            PatchOffset(0, root);
            return bytes.ToArray();
        }
    }

    public static byte[] Encode(string[] read, string[] write, string[] deny) {
        var buffer = new Buffer();
        // Root field indices: version, capabilities, win32k, UI, write,
        // read, deny, network policy, enumerate. Version is an inline struct.
        int root = buffer.Table(28, 4, 8, 0, 0, 12, 16, 20, 24, 0);
        buffer.PatchByte(root + 4, 1); // Schema major 1.
        buffer.PatchByte(root + 6, 1); // Schema minor 1.
        buffer.PatchOffset(root + 8,
            buffer.String("internetClient,internetClientServer,privateNetworkClientServer,networkLoopback"));
        buffer.PatchOffset(root + 12, buffer.Strings(write));
        buffer.PatchOffset(root + 16, buffer.Strings(read));
        buffer.PatchOffset(root + 20, buffer.Strings(deny));

        // The reserved peer name is part of the PSEC host-loopback contract.
        // NetworkPolicy: proxy, egress, peer, ingress.
        int network = buffer.Table(16, 0, 4, 8, 12);
        buffer.PatchOffset(root + 24, network);
        buffer.PatchOffset(network + 8, buffer.String("MXC-Loopback"));
        int egress = buffer.Table(8, 4, 0, 0);
        buffer.PatchByte(egress + 4, 1); // FilterAction.allow.
        buffer.PatchOffset(network + 4, egress);
        int ingress = buffer.Table(8, 4, 5);
        buffer.PatchByte(ingress + 4, 1); // Allow inbound connections.
        buffer.PatchByte(ingress + 5, 1); // Allow host loopback.
        buffer.PatchOffset(network + 12, ingress);
        return buffer.Finish(root);
    }
}
