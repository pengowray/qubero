/*
 * SelectionData.java
 *
 * Created on 18 November 2002, 10:42
 *
 * Creates a Data object by combining a selection and data.
 *
 * does not clone either data or selection.
 *
 * xxx: ignores holes in the data, squishing all data together. requires new
 * superclass: MaskedData
 *
 */
import java.io.*;

/**
 *
 * @author  Peter Halasz
 */
public class SelectionData extends Data {
    //xxx: should these be cloned?.. probably
    LongListSelectionModel lsm;
    Data data;
    
    /** Creates a new instance of SelectionData */
    public SelectionData(LongListSelectionModel lsm, Data data) {
        this.lsm = lsm;
        this.data = data;
    }
    
    public InputStream getDataStream() {
        return new SelectionStream();
    }
    
    public long getLength() {
        return lsm.getMaxSelectionIndex() - lsm.getMinSelectionIndex();
    }
    
    public long getStart() {
        //xxx: may return -1.. is that wrong?
        return lsm.getMinSelectionIndex();        
    }
    
    class SelectionStream extends InputStream {
        long pos = getStart();
        
        //xxx: use getSelectionIndexes() instead
        public int read() throws IOException {
            if (pos == -1)
                return -1;
            
            while (!lsm.isSelectedIndex(pos)) {
                if (pos > lsm.getMaxSelectionIndex())
                    return -1;
                pos++;
            }
            
            return (int)(data.getBytes(pos,1)[0]); //xxx: use a stream from data too.
        }
        
    }
}
