/*
 * Created on Dec 30, 2003
 *
 * To change the template for this generated file go to
 * Window - Preferences - Java - Code Generation - Code and Comments
 */
package net.pengo.propertyEditor;

import net.pengo.resource.MagicNumberResource;

/**
 * @author Smiley
 */
public class MagicNumberForm  extends AbstractResourcePropertiesForm  {
    private MagicNumberResource magic;
    
    public MagicNumberForm(MagicNumberResource magic) {
        super();
        this.magic = magic;
    }
    
    /* (non-Javadoc)
     * @see net.pengo.propertyEditor.AbstractResourcePropertiesForm#getMenu()
     */
    protected PropertyPage[] getMenu() {
        return new PropertyPage[] {
                new NamePage(magic, this),        
                new ResourceSelectorPage(magic.getJPointers()[0], this),
                new ResourceSelectorPage(magic.getJPointers()[1], this),
                new QNodePage(magic)
        };
    }
}
