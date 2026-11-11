using System;
using System.IO;
using System.Threading;

// Compiled with the production helper only by the Windows Rust test. This
// partial class exercises the actual private frame readers without ConPTY.
public static partial class JecodePty {
    static byte[] TestFrame(byte operation, byte[] payload) {
        var frame = new byte[5 + payload.Length];
        frame[0] = operation;
        Buffer.BlockCopy(BitConverter.GetBytes(payload.Length), 0, frame, 1, 4);
        Buffer.BlockCopy(payload, 0, frame, 5, payload.Length);
        return frame;
    }

    static byte[] InputFrame(ulong generation, byte value) {
        var payload = new byte[9];
        Buffer.BlockCopy(BitConverter.GetBytes(generation), 0, payload, 0, 8);
        payload[8] = value;
        return TestFrame(1, payload);
    }

    static byte[] InterruptFrame(ulong generation) {
        return TestFrame(3, BitConverter.GetBytes(generation));
    }

    static MemoryStream InputFrames(params byte[][] frames) {
        var bytes = new MemoryStream();
        foreach (byte[] frame in frames) bytes.Write(frame, 0, frame.Length);
        bytes.Position = 0;
        return bytes;
    }

    static void Expect(byte[] actual, params byte[] expected) {
        if (actual.Length != expected.Length)
            throw new Exception("Wrong input length: " + actual.Length);
        for (int i = 0; i < expected.Length; i++)
            if (actual[i] != expected[i])
                throw new Exception("Wrong input at " + i + ": " + actual[i]);
    }

    public static void TestSequence() {
        // A newer input frame can arrive on its pipe before the control frame.
        var sequence = new InputSequence();
        var output = new MemoryStream();
        Exception inputFailure = null;
        var input = new Thread(() => {
            try { InputLoop(InputFrames(InputFrame(1, (byte)'N')), output, sequence); }
            catch (Exception error) { inputFailure = error; }
        });
        input.IsBackground = true;
        input.Start();
        DateTime deadline = DateTime.UtcNow.AddSeconds(2);
        while ((input.ThreadState & ThreadState.WaitSleepJoin) == 0 && DateTime.UtcNow < deadline)
            Thread.Sleep(1);
        if ((input.ThreadState & ThreadState.WaitSleepJoin) == 0)
            throw new Exception("Future input did not wait for its control frame");
        Expect(output.ToArray());
        ControlLoop(InputFrames(InterruptFrame(1)), output, new ConsoleState(), sequence);
        if (!input.Join(1000)) throw new Exception("Future input remained blocked");
        if (inputFailure != null) throw new Exception("Future input failed", inputFailure);
        Expect(output.ToArray(), 3, (byte)'N');

        // After the interrupt, stale input still waiting in the other pipe is
        // discarded and input from the new generation follows Ctrl-C.
        sequence = new InputSequence();
        output = new MemoryStream();
        ControlLoop(InputFrames(InterruptFrame(1)), output, new ConsoleState(), sequence);
        InputLoop(InputFrames(InputFrame(0, (byte)'S'), InputFrame(1, (byte)'N')),
            output, sequence);
        Expect(output.ToArray(), 3, (byte)'N');
    }
}

public static class JecodePtySequenceTests {
    public static int Main() {
        try { JecodePty.TestSequence(); return 0; }
        catch (Exception error) { Console.Error.WriteLine(error); return 1; }
    }
}
