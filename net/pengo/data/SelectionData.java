/*
 * SelectionData.java
 *
 * Created on 18 November 2002, 10:42
 *
 * Creates a Data object by combining a selection and data.
 *
 * clones neither data nor selection.
 *
 * FIXME: ignores holes in the data, squishing all data together. requires new
 * superclass: MaskedData
 *
 */
package net.pengo.data;

import net.pengo.selection.*;


import java.io.*;

/**
 *
 * @author  Peter Halasz
 */
public class SelectionData extends Data {
    //should these be cloned?.. probably, but leave up to client
    LongListSelectionModel lsm;
    Data data;
    
    /** Creates a new instance of SelectionData */
    public SelectionData(LongListSelectionModel lsm, Data data) {
        this.lsm = lsm;
        this.data = data;
    }
    
    public InputStream dataStream() {
        return new SelectionStream();
    }
    
    public long getLength() {
        return lsm.getSelectionIndexes().length;
    }
    
    public long getStart() {
        //FIXME: may return -1.. is that wrong?
        return lsm.getMinSelectionIndex();
    }
    
    class SelectionStream extends InputStream {
		long[] sel = lsm.getSelectionIndexes();
        int pos = 0;
		
        public int read() throws IOException {
            if (sel.length == 0 || pos >= sel.length) {
				return -1;
			}
			long sp = sel[pos];
			int r = (int)(data.readByteArray(sp,1)[0]); //FIXME: use a stream from data too.
            //System.out.println("reading byte "+ sp +" of " + sel.length + " from a selection stream: " + r );
			pos++;
            return r;
        }
        
    }
}
