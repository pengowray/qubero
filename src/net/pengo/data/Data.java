package net.pengo.data;
import net.pengo.app.*;

import java.io.*;

/**
 * Data is a fixed length chunk of data. e.g. a binary file, or an area of memory.
 */
public abstract class Data implements Comparable {
    // xxy: replace with ByteBuffer ? - no doesn't support long indexes
    abstract public long getLength();
    
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
		    System.out.println("couldn't skip good");
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
    
}
