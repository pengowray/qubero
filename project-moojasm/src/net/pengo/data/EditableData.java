/*
 * EditableData.java
 *
 * Created on 7 September 2002, 09:02
 */

/**
 *
 * @author  administrator
 */
package net.pengo.data;

import net.pengo.selection.*;
import java.util.*;

public abstract class EditableData extends Data {
    Set datalisteners = new HashSet() ; //DataListener's
    
    public void addDataListener(DataListener l) {
	datalisteners.add(l);
    }
    
    public void removeDataListener(DataListener l) {
	datalisteners.remove(l);
    }
    
    protected void fireDataUpdated(Object source, DiffData.Mod mod){
	for (Iterator it = datalisteners.iterator(); it.hasNext(); ) {
	    DataListener l = (DataListener)it.next();
	    DataEvent e = new DataEvent(source, mod);
	    l.dataUpdate(e);
	}
    }
    
    public abstract void delete(long offset, long length);
    
    public abstract void delete(LongListSelectionModel selection);
    
    public void insert(long offset, Data data) {
        data = data.shiftStart(offset - data.getStart());
        insert(data);
    }

    public abstract void insert(Data data);
    
    public void overwrite(long offset, Data data) {
        data = data.shiftStart(offset - data.getStart());
        overwrite(data);
    }

    public abstract void overwrite(Data data);
    
    public void insertReplace(LongListSelectionModel selection, Data newData) {
	
	//use overwrite if possible
	if (selection.getSegmentCount() == 1 &&
	    newData.getLength() == selection.getMaxSelectionIndex() - selection.getMinSelectionIndex() +1) {
	    //fixme: if newData is has holes, this may stuff up
	    overwrite(selection.getMinSelectionIndex(), newData);
	    return;
	}
	
        delete(selection);
	//fixme: multiple selection insertion?
        insert(selection.getAnchorSelectionIndex(), newData);
    }
}
