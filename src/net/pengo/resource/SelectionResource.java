/*
 * SelectionResource.java
 *
 * Created on 4 September 2002, 19:40
 */

/**
 *
 * @author  Peter Halasz
 */
package net.pengo.resource;
import net.pengo.app.OpenFile;
import net.pengo.data.Data;
import net.pengo.data.SelectionData;
import net.pengo.dependency.QNode;
import net.pengo.selection.LongListSelectionEvent;
import net.pengo.selection.LongListSelectionListener;
import net.pengo.selection.LongListSelectionModel;

//acts as a wrapper for LongListSelectionModel.

abstract public class SelectionResource extends QNodeResource implements LongListSelectionListener, QNode {

    protected OpenFile openFile;
    
     abstract public LongListSelectionModel getSelection();

     abstract public SelectionData getSelectionData();
     
     public SelectionResource(OpenFile openFile){
         this.openFile = openFile;
     }
     
     public OpenFile getOpenFile(){
         return openFile;
     }

     /**
     * Called whenever the value of the selection changes.
     * @param e the event that characterizes the change.
     *
     */
    public void valueChanged(LongListSelectionEvent e) {
	if (e.getValueIsAdjusting()) {
	    return;
	}
        
	updated();
    }
    
    abstract public void updated();
    

    public String valueDesc() {
        return getSelection().toString();
    }
    
    public void insertReplace(Data newdata) {
        openFile.getEditableData().insertReplace(getSelection(), newdata);
    }
    
    public void editProperties() {
        // TODO Auto-generated method stub
        System.out.println("editing... not really");
    }
    
    /** makes the selection active (selected) in openfile. */
    public void makeActive() {
	    LongListSelectionModel selection = (LongListSelectionModel)getSelection().clone();
	    openFile.setSelectionModel(selection);
    }
    
    //FIXME:; reimplement?
    /*
     public boolean equals(SelectionResource o) {
     if (o == this)
     return true;
     
     openFile.getSelectionModel().eq
     return (sel.getStart() == o.sel.getStart() && sel.getLength() == o.sel.getLength());
     }
     
     public boolean equals(Object o) {
     if (o == this)
     return true;
     
     if (o instanceof SelectionResource) {
     return equals((SelectionResource)o);
     }
     
     return false;
     }
     */
}
