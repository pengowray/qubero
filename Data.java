import java.io.*;

/** 
 * RawData is a fixed length chunk of data. e.g. a binary file, or an area of memory.
 */
abstract class Data implements Comparable {
    // xxx: replace with ByteBuffer ?
    abstract public long getLength();
    
    /**
     * fill byte[] with data, starting at offset of the data itself (not the array).
     * @ returns the total number of bytes read into the buffer, or -1 if there 
     * is no more data because the end of the data-chunk has been reached. 
     */
    //abstract public int getData(byte[] b, int off);
    
    //XXX: throw out of bounds thing
    public Data getSelection(long start, long length) {
        return new TransparentData(this, start, length);
    }
    
    public TransparentData getTransparentData() {
        return getTransparentData(getStart(), getLength());
    }

    public TransparentData getTransparentData(long start, long length) {
        return new TransparentData(this, start, length);
    }
    
    /** shift is how far to shift the view of the source data. used for deletes */
    public Data getSourceShiftedSelection(long start, long length, long shiftSource) {
        return new ShiftedData(start, length, shiftSource, this);
    }
    
    public Data getStartShiftedSelection(long start, long length, long shiftStart) {
        return new ShiftedData(this, start, length, shiftStart);
    }
    
    public Data getStartShiftedSelection(long shiftStart) {
        return new ShiftedData(this, getStart(), getLength(), shiftStart);
    }    

    /** first piece of data is where? */
    public long getStart() {
        return 0;
    }
    
    //public Chunk getMultpleSelection(...);    
    
    public String toString() {
        return "raw data";
    }
        
    public String getType() {
        return "Raw data";
    }

    // start is relative to the board.
    abstract public InputStream getDataStream(long start, long length);

    // offset is relative to the start of the object.
    public InputStream getDataStream(long offset) throws IOException {
        InputStream i = getDataStream();
        i.skip((int)offset); // precision!
        return i;
    }

    abstract public InputStream getDataStream();
    
    public byte[] getDataStreamAsArray() { // throws IOException 
        try {
            if (getLength() > Integer.MAX_VALUE){
                throw new IllegalArgumentException("Data too large to fit into an array.");
            }
            byte[] b = new byte[(int)getLength()];
            getDataStream().read(b);
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
     * XXX: Note: this class has a natural ordering that is
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
        System.out.println("comparing data with a..." + obj.getClass() + " ok! " + obj);
        return -(obj.compareTo(this));
    }

    public int compareTo(Data obj) {
        long s = obj.getStart();
        if (getStart() < s) { // xxx: will ShiftedData's getStart method be called?
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
    
}
