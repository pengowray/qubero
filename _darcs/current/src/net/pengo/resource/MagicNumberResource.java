/**
 * MagicNumberResource.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.resource;

import net.pengo.pointer.JavaPointer;
import net.pengo.pointer.SmartPointer;
import net.pengo.propertyEditor.ResourceForm;

public class MagicNumberResource extends DefinitionResource implements AddressedResource {

    // the selection within the template which should match the magic number
    public final SmartPointer sel = new JavaPointer("net.pengo.resource.SelectionResource");
    
    // the magic number itself
    public final SmartPointer magic = new JavaPointer("net.pengo.resource.SelectionResource"); 
    
    public MagicNumberResource(SelectionResource sel){
        this(sel, sel);
    
    }
            
    /**
     * @param openFile
     */
    public MagicNumberResource(SelectionResource sel, SelectionResource magic) {
        this.sel.setName("selection");
        this.sel.addSink(this);
        this.sel.setValue(sel);
        
        this.magic.setName("magic number");
        this.magic.addSink(this);
        this.magic.setValue(magic);
    }

    /* (non-Javadoc)
     * @see net.pengo.resource.DefinitionResource#editProperties()
     */
    public void editProperties() {
        new ResourceForm(this).show();
    }

    /* (non-Javadoc)
     * @see net.pengo.resource.QNodeResource#getSources()
     */
    public Resource[] getSources() {
        return new Resource[] { sel, magic};
    }
    
    public JavaPointer[] getJPointers() {
        return new JavaPointer[]{(JavaPointer)sel, (JavaPointer)magic};
    }
    
    /* (non-Javadoc)
     * @see net.pengo.resource.AddressedResource#getSelectionResource()
     */
    public SelectionResource getSelectionResource() {
        return (SelectionResource)sel.evaluate();
    }

    /* (non-Javadoc)
     * @see net.pengo.resource.AddressedResource#setSelectionResource(net.pengo.resource.SelectionResource)
     */
    public void setSelectionResource(SelectionResource selRes) {
        sel.setValue(selRes);
    }

    public void doubleClickAction() {
        super.doubleClickAction();
        getSelectionResource().makeActive();
    }
}

