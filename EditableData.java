/*
 * EditableData.java
 *
 * Created on 7 September 2002, 09:02
 */

/**
 *
 * @author  administrator
 */
public abstract class EditableData extends Data {
    
    public abstract void delete(long offset, long length);
    
    public abstract void delete(LongListSelectionModel selection);
    
    public void insert(long offset, Data data) {
        data = data.getStartShiftedSelection(offset - data.getStart());
        insert(data);
    }

    public abstract void insert(Data data);
    
    public void overwrite(long offset, Data data) {
        data = data.getStartShiftedSelection(offset - data.getStart());
        overwrite(data);
    }

    public abstract void overwrite(Data data);
    
    public void insertReplace(LongListSelectionModel selection, Data newData) {
        //delete(oldData.getStart(), oldData.getLength());
        delete(selection);
        insert(selection.getAnchorSelectionIndex(), newData); //xxx: anchor or lead?
    }
}
