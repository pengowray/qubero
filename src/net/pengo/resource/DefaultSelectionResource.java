/**
 * DefaultSelectionResource.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.resource;

import net.pengo.app.OpenFile;
import net.pengo.data.SelectionData;
import net.pengo.selection.LongListSelectionModel;

public class DefaultSelectionResource extends SelectionResource
{
    private LongListSelectionModel sel;
    private SelectionData selData; // cache thing

    public DefaultSelectionResource(OpenFile openFile, LongListSelectionModel sel) {
            super(openFile);
            this.sel = sel;
            getSelection().addLongListSelectionListener(this);
	}

	public Resource[] getSources()
	{
		//xxx: umm.. this acts as a primative?
		return new Resource[]{};
	}
	
	public void setSelection(LongListSelectionModel sel) {
         selData = null;
         this.sel = sel;

         //xxx: alert the others
     }

     public SelectionData getSelectionData() {
        if (selData == null)
            selData = new SelectionData(getSelection(), openFile.getData());
            
        return selData;
    }
     
    public LongListSelectionModel getSelection() {
        return sel;
    }
    
    public void updated() {
        selData = null;
    }
    
}

