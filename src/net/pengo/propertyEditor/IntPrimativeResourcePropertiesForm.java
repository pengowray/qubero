/**
 * IntPrimativeResourcePropertiesForm.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.propertyEditor;

import net.pengo.resource.IntPrimativeResource;

public class IntPrimativeResourcePropertiesForm extends AbstractResourcePropertiesForm
{
    private IntPrimativeResource intRes;
    
    public IntPrimativeResourcePropertiesForm(IntPrimativeResource intRes) {
        super();
        this.intRes = intRes;
    }
    
    // override this.
    protected PropertyPage[] getMenu() {
        return new PropertyPage[] {
            new NamePage(intRes, this),
            new IntValuePage(intRes, this),
			new QNodePage(intRes)
        };
    }
    
}

