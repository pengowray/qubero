package net.pengo.data;
import net.pengo.app.*;

import java.io.*;
import net.pengo.bitSelection.BitCursor;
import net.pengo.bitSelection.BitSegment;

/**
 * Data is a fixed length chunk of data. e.g. a binary file, or an area of memory.
 */
public abstract class Data implements Comparable {
    // xxy: replace with ByteBuffer ? - no doesn't support long indexes
    abstract public long getLength();
    
    public BitCursor getBitLength() {
        return new BitCursor(getLength(), 0);
    }
    
    /**
     * fill byte[] with data, starting at offset of the data itself (not the array).
     * @ returns the total number of bytes read into the buffer, or -1 if there
     * is no more data because the end of the data-chunk has been reached.
     */
    //abstract public int getData(byte[] b, int off);
    
    //FIXME (maybe): remove this method!.. replace with a real selectionModel?
    public Data getSelection(long start, long length) {
        //System.out.println("Using deprecated method maybe: public Data getSelection(long start, long length) \n of Data");
        return new TransparentData(this, start, length);
    }
    
    //FIXME: NYI
	//FIXME: use "new SelectionData" instead
    //public Data getSelectionData(SelectionModel sm) {
    //    return new DiffData(sm);
    //}
    
    public Data shiftStart(long shiftStart) {
        return new ShiftedData(this, getStart(), getLength(), shiftStart);
    }
   
    public TransparentData subdata(long start, long length) {
        return new TransparentData(this, start, length);
    }
    
    /** shift is how far to shift the view of the source data. used for deletes */
    // was: getSourceShiftedSelection()
    public Data slideSource(long start, long length, long shiftSource) {
        return new ShiftedData(start, length, shiftSource, this);
    }
    
    public Data shiftStart(long start, long length, long shiftStart) {
        return new ShiftedData(this, start, length, shiftStart);
    }

    /** first piece of data is where? */
    public long getStart() {
        return 0;
    }
    
    //public Chunk getMultpleSelection(...);
    
    public String toString() {
        return "data";
    }
        
    public String getType() {
        return "data";
    }

    //////////////////////// cut
    /*
     * too many convinience methods.. remove some:
     */
    // start is relative to the board.
    // FIXME: rename to just "dataStream()"
    public InputStream getDataStream(long start, long length)  throws IOException {
        //FIXME: check for errors
        return getDataStream(start - getStart());
    }

    // offset is relative to the start of the object.
    // FIXME: rename to just "dataStream()"
    public InputStream getDataStream(long offset) throws IOException {
        InputStream i = dataStream();
        i.skip((int)offset); // precision!
        return i;
    }
    
    ///////////////////// remove to here

    /** @returns the main data stream */
    abstract public InputStream dataStream() throws IOException;
    
    public byte[] readByteArray() throws IOException {
        try {
            if (getLength() > Integer.MAX_VALUE){
                throw new IllegalArgumentException("Data too large to fit into an array.");
            }
            byte[] b = new byte[(int)getLength()];
            dataStream().read(b);
            return b;
        } catch (IOException e) {
            e.printStackTrace();
            return new byte[(int)getLength()];
            // XXX throw
        }
    }
    
    public byte[] readByteArray(long start, int length) throws IOException {
        try {
            byte[] b = new byte[length];
            InputStream stream = dataStream();
			long toSkip = start - getStart();
            if (toSkip >= 0) { // start >= getStart()
                long skipped = stream.skip(toSkip);
                if (skipped != toSkip) {
                    //FIXME: error i guess
                	//throw new Exception("skip failed."); //System.out.println("couldn't skip good");
                	//new Exception("skip failed. start:" + start + " length:" + length + " data_length():" + getLength()).printStackTrace();
                	return new byte[length];
//                	throw new Exception("skip failed."); //System.out.println("couldn't skip good");
                }
            } else {
                throw new IOException("tried to getBytes from before the start");
            }
            stream.read(b);
            return b;
        } catch (IOException e) {
            e.printStackTrace();
            return new byte[(int)getLength()];
            // XXX throw
        }
    }

    /** Compares this object with the specified object for order.  Returns a
     * negative integer, zero, or a positive integer as this object is less
     * than, equal to, or greater than the specified object.<p>
     *
     * FIXME: Note: this class has a natural ordering that is
     * inconsistent with equals. (only slightly.. )
     *
     * @param   o the Object to be compared.
     * @return  a negative integer, zero, or a positive integer as this object
     * 		is less than, equal to, or greater than the specified object.
     *
     * @throws ClassCastException if the specified object's type prevents it
     *         from being compared to this Object.
     *
     */
    public int compareTo(Object o) {
        if (o == null)
            throw new NullPointerException();
        
        if (o instanceof Cursor) {
            return compareTo((Cursor)o);
        }
        if (o instanceof Data) {
            return compareTo((Data)o);
        }

        throw new ClassCastException("Tried to compare " + o.getClass() + " with this " + this.getClass());
    }

    public int compareTo(Cursor obj) {
        //System.out.println("comparing data with a..." + obj.getClass() + " ok! " + obj);
        return -(obj.compareTo(this));
    }

    public int compareTo(Data obj) {
        long s = obj.getStart();
        if (getStart() < s) { // FIXME: will ShiftedData's getStart method be called?
            return -1;
        } else if (getStart() > s) {
            return 1;
        } else {
            // equal starts
            long ol = obj.getLength();
            long tl = getLength();
            if (tl < ol) {
                return -1;
            } else if (tl > ol) {
                return 1;
            } else {
                if (obj.equals(this)) {
                    return 0;
                } else if (this.hashCode() < obj.hashCode()) {
                    return -1;
                } else {
                    return 1;
                }
            }
        }
    }
    
    public boolean equals(Cursor obj) {
        return false;
    }
    
    public int readBitsToInt(BitSegment segment) throws IOException {
        int BIT_COUNT = Integer.bitCount(-1); // 32
        
        int unitSize = (int)segment.getLength().toBits();
        
        long xByte = segment.firstIndex.getByteOffset(); // set initial byte
        int xBit = segment.firstIndex.getBitOffset(); 
        int rightShift = BIT_COUNT - unitSize - xBit; // how much data will need to be right shifted before being returned
        
        int mask = (int)(-1) >>> xBit // leftGap.. -1 = 0xFFFFFFFF
                 & (int)(-1) << rightShift; //rightGap;

        int bytesInUnit = 1+ ((unitSize+xBit)/8); // how many bytes will we need to read

        //System.out.println("reading " + xByte);
        byte[] bytes = readByteArray(xByte, bytesInUnit);

        int readBytes = 0;
        for (int b=0; b<bytesInUnit; b++) {
            readBytes |= (int)(bytes[b]) << (BIT_COUNT - 8 - (b*8));
        }

        int maskedBytes = readBytes & mask;

        int rShiftedBytes = maskedBytes >>> rightShift;

        /*
        System.out.println("xByte=" + xByte + " xBit=" + xBit + " bytesInUnit=" + bytesInUnit + " mask=" + Integer.toBinaryString(mask) + "b" + 
            " rightShift=" + rightShift + " bytesInUnit=" + bytesInUnit + " readBytes=" + Integer.toHexString(readBytes) + 
            "h, " + Integer.toBinaryString(readBytes) + "b maskedBytes=" + Integer.toBinaryString(maskedBytes) + "b rShiftedBytes=" + Integer.toBinaryString(rShiftedBytes) + "b");
        */

        return rShiftedBytes;
    }
    
    /* unitSize = number of bits to return in each int
       startUnit = first unit to return (assuming all binary is in unitSize units
     * bitOffset, can be any number
     *
     *fixme: maybe startByteOffset would be more useful than startUnit
     *
     *fixme: optimise for special cases like unitSize = 4 or 8
     *
     * @depricated use readBitsToInt //how do i do this relaly?
     */
    public int[] readIntArray(int unitSize, long startUnit, int bitOffset, int unitCount) throws IOException {
        //fixme: no checking
        //long xByte = (long) (  (startUnit * unitSize + bitOffset) / 8 ); // set initial byte
        //int xBit = (int) ( (startUnit * unitSize + bitOffset) % 8 ); // set initial bit
        int BIT_COUNT = Integer.bitCount(-1); // 32
        
        //int maxBytesUnit = // too difficult to calculate and better ways.... 1 gives 1, 2-9 gives 2, 10+ gives 3.. 
        //System.out.println("unitSize=" + unitSize + " startUnit=" + startUnit + " unitCount=" + unitCount);
        
        int[] returnArray = new int[unitCount];
        
        for (int i=0; i < unitCount; i++) {
            long xByte = (long) (  ((startUnit + i) * unitSize + bitOffset) / 8 ); // set initial byte
            int xBit = (int) ( ((startUnit + i) * unitSize + bitOffset) % 8 ); // set initial bit
            
            //bits remaining in first byte = 8-xBit 
            //loc of first bit is unitSize
            //bits left to extract = unitSize
            
            int rightShift = BIT_COUNT - unitSize - xBit; // how much data will need to be right shifted before being returned
            
            int mask = (int)(-1) >>> xBit // leftGap.. -1 = 0xFFFFFFFF
                     & (int)(-1) << rightShift; //rightGap;
            
            int bytesInUnit = 1+ ((unitSize+xBit)/8); // how many bytes will we need to read
            
            byte[] bytes = readByteArray(xByte, bytesInUnit);
            
            int readBytes = 0;
            for (int b=0; b<bytesInUnit; b++) {
                readBytes |= (int)(bytes[b]) << (BIT_COUNT - 8 - (b*8));
            }
            
            int maskedBytes = readBytes & mask;
            
            int rShiftedBytes = maskedBytes >>> rightShift;
            
            /*
            System.out.println("xByte=" + xByte + " xBit=" + xBit + " bytesInUnit=" + bytesInUnit + " mask=" + Integer.toBinaryString(mask) + "b" + 
                " rightShift=" + rightShift + " bytesInUnit=" + bytesInUnit + " readBytes=" + Integer.toHexString(readBytes) + 
                "h, " + Integer.toBinaryString(readBytes) + "b maskedBytes=" + Integer.toBinaryString(maskedBytes) + "b rShiftedBytes=" + Integer.toBinaryString(rShiftedBytes) + "b");
            */
            
            returnArray[i] = rShiftedBytes;
        }
        
        return returnArray;
    }
    
    
}
